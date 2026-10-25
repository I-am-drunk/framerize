//! Connect collector outcomes to the pinned SDK's final value positions.
use super::{network_address, network_hash, shared_worker_version, NetworkValueError,
    NormalizedWorkerValue, Value};

pub enum CollectorResult {
    SharedWorker(Value),
    Network(Value),
    NetworkHash(Value),
}

#[derive(Debug, PartialEq, Eq)]
pub enum CollectorError {
    MissingPosition(usize),
    SharedWorker(NormalizedWorkerValue),
    Network(NetworkValueError),
}

/// Apply one completed collector result to an already supplied value array.
///
/// SDK3337eba7 assigns Q[242] at 282619, Q[469] through Brn/Cr at 284185,
/// and Q[473] at 284204. Each variant models one assignment. Apply the hash
/// before the address to follow the network callback's order. A refusal of
/// one assignment leaves all positions as they were before that assignment.
/// This never extends an incomplete array or declares collection/readiness.
pub fn apply_collector_result(
    values: &mut [Value], result: CollectorResult,
) -> Result<(), CollectorError> {
    let position = match &result {
        CollectorResult::SharedWorker(_) => 242,
        CollectorResult::Network(_) => 473,
        CollectorResult::NetworkHash(_) => 469,
    };
    let target = values.get_mut(position)
        .ok_or(CollectorError::MissingPosition(position))?;
    let value = match result {
        CollectorResult::SharedWorker(value) => shared_worker_version(value)
            .map_err(CollectorError::SharedWorker)?,
        CollectorResult::Network(value) => network_address(value)
            .map_err(CollectorError::Network)?,
        CollectorResult::NetworkHash(value) => network_hash(value)
            .map_err(CollectorError::Network)?,
    };
    *target = value;
    Ok(())
}
