//! Native runtime library, separate from the browser application.
pub mod account;
pub mod authorization;
pub mod request;
pub mod transport;
pub mod worker;
mod service;

/// Validate explicit worker inputs without executing a worker or opening accounts.
pub fn check_config(path: &str, v2: bool) -> Result<(), &'static str> {
    if v2 { worker::config::load_v2(path).map(|_| ()) }
    else { worker::config::load(path).map(|_| ()) }
}

#[cfg(test)]
mod test_support;
