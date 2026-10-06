//! /ai/v3/responses/ — OpenAI Responses format via OpenRouter.
//! Free non-streaming (verified 2026-10-05). Codex-native free path.
use super::{Metering, Route};
use crate::proxy::Dialect;

pub const ROUTE: Route = Route {
    id: "openrouter-responses",
    path: "/ai/v3/responses/",
    dialect: Dialect::OpenRouterResponses,
    metering: Metering::FreeNonStreaming,
    probe_model: "openai/gpt-4.1",
    primary: true,
};
