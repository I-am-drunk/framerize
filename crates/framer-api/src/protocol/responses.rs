//! Compatibility path for native callers of Responses preparation.
pub use framer_protocol::responses_input::*;

#[cfg(test)]
use serde_json::{json, Value};
#[cfg(test)]
#[path = "responses_namespace_tests.rs"]
mod namespace_tests;
