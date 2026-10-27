use crate::WireError;
use serde::{Deserialize, Serialize};

// Numeric JSON bytes need at most four encoded bytes each. The fixed Data
// envelope, including maximal decimal IDs, fits within the remaining 512.
pub const DATA_WIRE_RESERVE: u64 = (crate::SEGMENT_LIMIT * 4 + 512) as u64;
/// Charge a whole decoded segment allocation, including short/empty frames.
/// This matches the two fixed-size output slots; it does not charge history.
pub fn data_cost(wire_bytes: u64) -> Capacity {
    Capacity { wire_bytes, slots: 1, downstream_bytes: crate::SEGMENT_LIMIT as u64 }
}

/// Exclusive event frontiers: zero means that no event has completed.
#[derive(Default, Debug)]
pub struct Frontier { complete: u64, accepted: u64 }
impl Frontier {
    pub fn completed(&self) -> u64 { self.complete }
    pub fn accepted(&self) -> u64 { self.accepted }
    pub fn segment(&mut self, sequence: u64, last: bool) -> Result<(), WireError> {
        if sequence != self.complete { return Err(WireError::Credit); }
        if last { self.complete = self.complete.checked_add(1).ok_or(WireError::Credit)?; }
        Ok(())
    }
    pub fn acknowledge(&mut self, next: u64) -> Result<(), WireError> {
        if next < self.accepted || next > self.complete { return Err(WireError::Credit); }
        self.accepted = next;
        Ok(())
    }
}

#[cfg(test)]
#[path = "credit_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capacity {
    pub wire_bytes: u64,
    pub slots: u64,
    pub downstream_bytes: u64,
}
/// Three independent accounts. Moving an item does not return its credit.
pub struct Window { limit: Capacity, available: Capacity }
impl Window {
    pub fn new(limit: Capacity) -> Result<Self, WireError> {
        if limit.wire_bytes == 0 || limit.slots == 0 || limit.downstream_bytes == 0 {
            return Err(WireError::Credit);
        }
        Ok(Self { limit, available: limit })
    }
    pub fn available(&self) -> Capacity { self.available }
    pub fn reserve(&mut self, cost: Capacity) -> Result<(), WireError> {
        self.available = self.available.sub(cost)?;
        Ok(())
    }
    pub fn release(&mut self, returned: Capacity) -> Result<(), WireError> {
        let available = self.available.add(returned)?;
        self.limit.sub(available)?;
        self.available = available;
        Ok(())
    }
    pub fn outstanding(&self) -> Capacity {
        self.limit.sub(self.available).expect("validated credit window")
    }
}
impl Capacity {
    fn add(self, other: Self) -> Result<Self, WireError> {
        Ok(Self {
            wire_bytes: self.wire_bytes.checked_add(other.wire_bytes).ok_or(WireError::Credit)?,
            slots: self.slots.checked_add(other.slots).ok_or(WireError::Credit)?,
            downstream_bytes: self.downstream_bytes.checked_add(other.downstream_bytes).ok_or(WireError::Credit)?,
        })
    }
    fn sub(self, other: Self) -> Result<Self, WireError> {
        Ok(Self {
            wire_bytes: self.wire_bytes.checked_sub(other.wire_bytes).ok_or(WireError::Credit)?,
            slots: self.slots.checked_sub(other.slots).ok_or(WireError::Credit)?,
            downstream_bytes: self.downstream_bytes.checked_sub(other.downstream_bytes).ok_or(WireError::Credit)?,
        })
    }
}
