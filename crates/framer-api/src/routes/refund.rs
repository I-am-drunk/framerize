//! /ai/v3/refund/ — reverses a streamed charge by turnId. NO Castle layer
//! (garbage tokens pass) and unmetered. Quota ~2 grants/window per account.
//! The daemon auto-refunds after every metered turn.
use super::{Metering, Route};
use crate::proxy::Dialect;

pub const ROUTE: Route = Route {
    id: "refund",
    path: "/ai/v3/refund/",
    dialect: Dialect::OpenRouterMessages, // envelope-only; dialect unused
    metering: Metering::Unmetered,
    probe_model: "",
    primary: false,
};
