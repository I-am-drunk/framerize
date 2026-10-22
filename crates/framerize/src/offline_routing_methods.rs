use super::*;
use framer_core::convert::{AssembledResponse, AssembledResponses};

fn payload_ok(payload: &Value, streaming: bool) -> bool {
    payload.is_object() && payload.get("stream").is_none_or(|v| v.is_boolean())
        && (payload["stream"].as_bool() == Some(true)) == streaming
}

impl OwnedInvocation for Prepared {
    fn raw(self: Box<Self>, payload: Value) -> port::InvocationRun<port::RawResult> {
        let valid = matches!(self.request.route, port::Route::Raw(_)) && payload_ok(&payload, false);
        running::launch(self, payload, valid, running::Control::new(), |call, payload| call.raw(payload))
    }

    fn messages(self: Box<Self>, payload: Value)
        -> port::InvocationRun<port::Completed<AssembledResponse>> {
        let valid = self.request.route == port::Route::Messages && payload_ok(&payload, false);
        running::launch(self, payload, valid, running::Control::new(), |call, payload| call.messages(payload))
    }

    fn responses(self: Box<Self>, payload: Value)
        -> port::InvocationRun<port::Completed<AssembledResponses>> {
        let valid = self.request.route == port::Route::Responses && payload_ok(&payload, false);
        running::launch(self, payload, valid, running::Control::new(), |call, payload| call.responses(payload))
    }

    fn raw_stream(self: Box<Self>, payload: Value, limits: port::RawStreamLimits) -> port::RawStreamRun {
        let valid = matches!(self.request.route, port::Route::Raw(_)) && payload_ok(&payload, true)
            && limits.validate().is_ok() && self.shared.streaming.is_some_and(|p| p.raw == limits);
        let control = running::Control::new();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let response = running::launch(self, payload, valid, control.clone(), move |call, payload| {
            let inner = call.raw_stream(payload, limits);
            let _ = sender.send(inner.finished);
            inner.response
        });
        let cleanup = response.cleanup.clone();
        let finished = async move {
            match receiver.await {
                Ok(finished) => finished.await,
                Err(_) => {
                    let _ = cleanup.await;
                    port::RawStreamCompletion { result: Err(control.failure()), headers: None,
                        emitted_bytes: 0, elapsed: std::time::Duration::ZERO }
                }
            }
        }.boxed().shared();
        port::RawStreamRun { response, finished }
    }

    fn composed_stream(self: Box<Self>, payload: Value, options: port::ComposedStreamOptions)
        -> port::ComposedStreamRun {
        composed(self, payload, options, None)
    }

    fn composed_stream_for_client(self: Box<Self>, payload: Value,
        options: port::ComposedStreamOptions, client_model: String) -> port::ComposedStreamRun {
        composed(self, payload, options, Some(client_model))
    }
}

fn composed(prepared: Box<Prepared>, payload: Value, options: port::ComposedStreamOptions,
    client_model: Option<String>) -> port::ComposedStreamRun {
    let valid = !matches!(prepared.request.route, port::Route::Raw(_))
        && payload_ok(&payload, true) && options.limits.validate().is_ok()
        && prepared.shared.streaming.is_some_and(|p| p.composed == options.limits);
    let control = running::Control::new();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let response = running::launch(prepared, payload, valid, control.clone(), move |call, payload| {
        let inner = match client_model {
            Some(model) => call.composed_stream_for_client(payload, options, model),
            None => call.composed_stream(payload, options),
        };
        let _ = sender.send(inner.finished);
        inner.response
    });
    let cleanup = response.cleanup.clone();
    let finished = async move {
        match receiver.await {
            Ok(finished) => finished.await,
            Err(_) => {
                let _ = cleanup.await;
                port::ComposedStreamCompletion { result: Err(control.failure()),
                    attempts: Arc::from([]), emitted_bytes: 0, elapsed: std::time::Duration::ZERO }
            }
        }
    }.boxed().shared();
    port::ComposedStreamRun { response, finished }
}
