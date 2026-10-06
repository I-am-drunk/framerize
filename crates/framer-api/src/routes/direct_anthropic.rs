//! /ai/v3/anthropic/messages/ — direct Anthropic Messages. Metered always.
//! ~60s non-streaming window (vs ~21.5s on the OpenRouter passthroughs).
use super::{Metering, Route};
use crate::proxy::Dialect;

pub const ROUTE: Route = Route {
    id: "direct-anthropic",
    path: "/ai/v3/anthropic/messages/",
    dialect: Dialect::AnthropicMessages,
    metering: Metering::AlwaysMetered,
    probe_model: "claude-haiku-4-5",
    primary: false,
};
