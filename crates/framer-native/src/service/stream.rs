//! Downstream SSE for a bounded native request. Upstream bodies are buffered.
use super::serving::Outcome;
use crate::request::complete_json::{CompletedJson, FreeRoute};
use crate::request::types::FailureKind;
use axum::{body::Body, response::Response};
use framer_core::convert::{self, ChainEvent, ClientDialect};
use futures::future::BoxFuture;
use std::{convert::Infallible, time::Duration};
use tokio::sync::mpsc;

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;

enum Composer {
    Responses(convert::ResponsesComposer),
    Messages(convert::SseComposer),
}

impl Composer {
    fn push(&mut self, event: ChainEvent) -> Vec<String> {
        match (self, event) {
            (Self::Responses(composer), ChainEvent::ResponsesItems { items, continues_text }) =>
                composer.push_items(&items, continues_text),
            (Self::Messages(composer), ChainEvent::Chunk(blocks)) => composer.push_chunk(&blocks),
            _ => Vec::new(),
        }
    }
}

struct StreamState {
    diagnostic_id: uuid::Uuid,
    result: BoxFuture<'static, Outcome>,
    events: mpsc::Receiver<ChainEvent>,
    composer: Option<Composer>,
    model: String,
    dialect: ClientDialect,
    opening: Option<Vec<String>>,
    events_open: bool,
    done: bool,
}

pub(super) fn response(result: BoxFuture<'static, Outcome>,
    events: mpsc::Receiver<ChainEvent>, route: FreeRoute, model: String,
    diagnostic_id: uuid::Uuid) -> Response {
    let id = format!("native_{}", uuid::Uuid::new_v4());
    let (dialect, mut composer) = match route {
        FreeRoute::Responses => (ClientDialect::OpenAiResponses,
            Composer::Responses(convert::ResponsesComposer::new(&model, &id))),
        FreeRoute::Messages => (ClientDialect::AnthropicMessages,
            Composer::Messages(convert::SseComposer::new(
                ClientDialect::AnthropicMessages, &id, &model))),
    };
    let opening = match &mut composer {
        Composer::Responses(composer) => composer.open(),
        Composer::Messages(_) => convert::stream_open(dialect, &model, &id),
    };
    let state = StreamState { diagnostic_id, result, events, composer: Some(composer), model: model.clone(), dialect,
        opening: Some(opening), events_open: true, done: false };
    let body = futures::stream::unfold(state, next);
    let mut response = Response::new(Body::from_stream(body));
    response.headers_mut().insert("content-type", "text/event-stream".parse().unwrap());
    response.headers_mut().insert("cache-control", "no-cache".parse().unwrap());
    response.headers_mut().insert("x-accel-buffering", "no".parse().unwrap());
    response
}

async fn next(mut state: StreamState) -> Option<(Result<bytes::Bytes, Infallible>, StreamState)> {
    if state.done { return None; }
    let heartbeat = tokio::time::sleep(Duration::from_secs(8));
    tokio::pin!(heartbeat);
    let frames = if let Some(opening) = state.opening.take() { opening } else { loop {
        tokio::select! {
            biased;
            outcome = &mut state.result => {
                super::http::record_completion(&outcome, state.diagnostic_id);
                let mut frames = Vec::new();
                while let Ok(event) = state.events.try_recv() {
                    if outcome.failure.is_none() && outcome.cleanup_confirmed {
                        frames.extend(state.composer.as_mut().unwrap().push(event));
                    }
                }
                frames.extend(finish(state.composer.take().unwrap(), outcome, &state.model, state.dialect));
                state.done = true;
                break frames;
            }
            event = state.events.recv(), if state.events_open => {
                match event {
                    Some(event) => break state.composer.as_mut().unwrap().push(event),
                    None => state.events_open = false,
                }
            }
            _ = &mut heartbeat => break vec![convert::ping_frame(state.dialect)],
        }
    }};
    Some((Ok(bytes::Bytes::from(frames.join("\n\n") + "\n\n")), state))
}

fn finish(composer: Composer, outcome: Outcome, model: &str, dialect: ClientDialect) -> Vec<String> {
    match (outcome.response, outcome.failure, outcome.cleanup_confirmed, composer) {
        (Some(CompletedJson::Responses(value)), None, true, Composer::Responses(composer)) =>
            composer.finish(&value),
        (Some(CompletedJson::Messages(value)), None, true, Composer::Messages(mut composer)) =>
            composer.finish(&value, model, true),
        (_, kind, clean, composer) => {
            let message = format!("{:?}",
                if clean { kind.unwrap_or(FailureKind::UnsupportedResponse) }
                else { FailureKind::Cleanup });
            match composer {
                Composer::Responses(composer) => composer.fail(&message),
                Composer::Messages(_) => convert::error_frames(dialect, &message),
            }
        }
    }
}
