//! Native collection support. No browser observations are synthesized here.
mod collation;
pub use collation::{CollationError, NetworkCollator};

#[cfg(test)]
#[path = "collation_tests.rs"]
mod tests;
