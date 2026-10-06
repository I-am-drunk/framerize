//! /ai/v3/chat/ — OpenAI chat/completions format via OpenRouter.
//! Free non-streaming (verified). Bucket-rotation partner of /messages/.
use super::{Metering, Route};
use crate::proxy::Dialect;

pub const ROUTE: Route = Route {
    id: "openrouter-chat",
    path: "/ai/v3/chat/",
    dialect: Dialect::OpenRouterChat,
    metering: Metering::FreeNonStreaming,
    probe_model: "openai/gpt-4.1",
    primary: true,
};
