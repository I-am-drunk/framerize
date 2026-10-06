//! /ai/v3/fetch/ — server-side URL fetch (the agent's web tool). No Castle
//! layer (garbage tokens pass). GET-only, SSRF-filtered. Utility, not an
//! inference path.
use super::{Metering, Route};
use crate::proxy::Dialect;

pub const ROUTE: Route = Route {
    id: "fetch",
    path: "/ai/v3/fetch/",
    dialect: Dialect::OpenRouterMessages, // envelope-only; dialect unused
    metering: Metering::Unmetered,
    probe_model: "",
    primary: false,
};
