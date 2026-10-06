//! /ai/v3/messages/ — Anthropic Messages format via OpenRouter.
//! THE free route: non-streaming bypasses the meter (verified at 6-decimal
//! ledger precision). The chain's primary carrier.
use super::{Metering, Route};
use crate::proxy::Dialect;

pub const ROUTE: Route = Route {
    id: "openrouter-messages",
    path: "/ai/v3/messages/",
    dialect: Dialect::OpenRouterMessages,
    metering: Metering::FreeNonStreaming,
    probe_model: "google/gemini-2.5-flash",
    primary: true,
};
