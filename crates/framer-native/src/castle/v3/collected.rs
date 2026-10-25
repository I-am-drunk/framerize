//! Connect collector outcomes to the pinned SDK's final value positions.
use super::{network_address, shared_worker_version, NetworkValueError,
    NormalizedWorkerValue, Value};

pub enum CollectorResult {
    SharedWorker(Value),
    Network(Value),
}

#[derive(Debug, PartialEq, Eq)]
pub enum CollectorError {
    MissingPosition(usize),
    SharedWorker(NormalizedWorkerValue),
    Network(NetworkValueError),
}

/// Apply one completed collector result to an already supplied value array.
///
/// SDK3337eba7 assigns Q[242] at 282619 and Q[473] at 284204. The surrounding
/// dispatch calls are identities. A refusal leaves all positions unchanged.
/// This never extends an incomplete array or declares collection/readiness.
pub fn apply_collector_result(
    values: &mut [Value], result: CollectorResult,
) -> Result<(), CollectorError> {
    let position = match &result {
        CollectorResult::SharedWorker(_) => 242,
        CollectorResult::Network(_) => 473,
    };
    let target = values.get_mut(position)
        .ok_or(CollectorError::MissingPosition(position))?;
    let value = match result {
        CollectorResult::SharedWorker(value) => shared_worker_version(value)
            .map_err(CollectorError::SharedWorker)?,
        CollectorResult::Network(value) => network_address(value)
            .map_err(CollectorError::Network)?,
    };
    *target = value;
    Ok(())
}
