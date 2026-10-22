//! Isolated integration test root. No provider registration or service startup.
extern crate offline_worker_owner;
mod observed_authorization;
mod one_shot_control;
mod offline_worker;
mod offline_source;
mod offline_serving;
mod offline_sessions;
mod offline_http;
mod offline_config;
mod offline_routing;
mod offline_request_context;
mod offline_routing_config;
mod startup;
use startup::start_prewarm;
