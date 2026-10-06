//! /ai/v3/openai/responses/ — direct OpenAI Responses (NOT via OpenRouter).
//! Metered always. Home of the gpt-6.x models (unprefixed ids). Protected by
//! the credit-bond floor in the server.
use super::{Metering, Route};
use crate::proxy::Dialect;

pub const ROUTE: Route = Route {
    id: "direct-responses",
    path: "/ai/v3/openai/responses/",
    dialect: Dialect::OpenAiResponses,
    metering: Metering::AlwaysMetered,
    probe_model: "gpt-6-sol",
    primary: true,
};
