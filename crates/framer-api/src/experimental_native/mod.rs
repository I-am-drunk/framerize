//! Optional native HTTP transport and response components.
//!
//! Enable the experimental-native Cargo feature to use this library path.
//! Enabling it does not select it in the application's request dispatch.
//! Callers own account/session binding, genuine SDK token provenance,
//! optional request-owner admission, deadlines and response cleanup. A completed response and
//! confirmed transport cleanup are separate outcomes.
//!
//! See README.md and provenance.json in this directory for scope and sources.

pub mod application;
pub mod consumer;
pub mod downstream_owner;
pub mod native_account;
pub mod native_http;
pub mod native_owner;
pub mod response_adapter;
pub mod sse_decoder;
pub mod terminal_response;

#[cfg(feature = "native-dispatch")]
pub mod authorization;
#[cfg(feature = "native-dispatch")]
pub mod observed_authorization;
#[cfg(feature = "native-dispatch")]
pub mod control_authorization;
#[cfg(feature = "native-dispatch")]
pub mod session_store;

#[cfg(all(test, feature = "native-dispatch"))]
mod session_lifecycle_tests;
#[cfg(all(test, feature = "native-dispatch"))]
mod authorization_tests;
