//! Connect collector outcomes to the pinned SDK's final value positions.
use super::{network_address, network_hash, network_related_address, network_protocols,
    shared_worker_version, NetworkValueError,
    NormalizedWorkerValue, Value};

pub enum CollectorResult {
    SharedWorker(Value),
    Network(Value),
    NetworkHash(Value),
    NetworkRelatedAddress(Value),
    NetworkProtocols(Value),
}

#[derive(Debug, PartialEq, Eq)]
pub enum CollectorError {
    MissingPosition(usize),
    SharedWorker(NormalizedWorkerValue),
    Network(NetworkValueError),
}

/// Apply the four network values in SDK order, consuming the observation once.
/// Conversion stops on the first error. Prior writes survive; later slots stay
/// unchanged. Timing and actual collection remain the caller's responsibility.
pub fn apply_network_result(values: &mut [Value], observation: Value)
    -> Result<(), CollectorError> {
    let mut results = super::network_result::network_assignments(observation);
    for position in [80, 469, 473, 659] {
        let target = values.get_mut(position)
            .ok_or(CollectorError::MissingPosition(position))?;
        *target = results.next().expect("four network assignments")
            .map_err(CollectorError::Network)?;
    }
    Ok(())
}

/// Apply one completed collector result to an already supplied value array.
///
/// SDK3337eba7 assigns Q[242] at 282619. Its network callback at 284106..284245
/// assigns related address Q[80], hash Q[469], address Q[473], then protocols
/// Q[659]. Each variant models one assignment. Apply them in that order,
/// stopping on the first error and retaining earlier writes. A refusal of
/// one assignment leaves all positions as they were before that assignment.
/// This never extends an incomplete array or declares collection/readiness.
pub fn apply_collector_result(
    values: &mut [Value], result: CollectorResult,
) -> Result<(), CollectorError> {
    let position = match &result {
        CollectorResult::SharedWorker(_) => 242,
        CollectorResult::Network(_) => 473,
        CollectorResult::NetworkHash(_) => 469,
        CollectorResult::NetworkRelatedAddress(_) => 80,
        CollectorResult::NetworkProtocols(_) => 659,
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
        CollectorResult::NetworkRelatedAddress(value) => network_related_address(value)
            .map_err(CollectorError::Network)?,
        CollectorResult::NetworkProtocols(value) => network_protocols(value)
            .map_err(CollectorError::Network)?,
    };
    *target = value;
    Ok(())
}
