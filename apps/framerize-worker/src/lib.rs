//! Policy worker. Runtime authorization and credentials stay in the kernel.
mod forward;
mod emitter;

#[cfg(test)]
mod runner_tests;
