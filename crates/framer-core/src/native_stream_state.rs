//! Conservative cumulative allocation admission for the opt-in composer.
//! Container reserves include slack/temporary reallocations; allocator metadata
//! and the native transport remain outside this local storage accounting.
use super::FailureKind;
use serde_json::Value;
use std::mem::size_of;

pub struct StateBudget { limit: usize, charged: usize }

pub fn value_bytes(value: &Value) -> Result<usize, FailureKind> {
    fn add(a: usize, b: usize) -> Result<usize, FailureKind> {
        a.checked_add(b).ok_or(FailureKind::BodyTooLarge)
    }
    fn walk(v: &Value, depth: usize) -> Result<usize, FailureKind> {
        if depth > 128 { return Err(FailureKind::BodyTooLarge); }
        let mut bytes = size_of::<Value>();
        match v {
            Value::String(s) => bytes = add(bytes, s.capacity())?,
            Value::Array(a) => {
                bytes = add(bytes, a.capacity().checked_mul(size_of::<Value>())
                    .ok_or(FailureKind::BodyTooLarge)?)?;
                for child in a { bytes = add(bytes, walk(child, depth + 1)?)?; }
            }
            Value::Object(o) => {
                bytes = add(bytes, o.len().checked_mul(4096)
                    .ok_or(FailureKind::BodyTooLarge)?)?;
                for (key, child) in o {
                    bytes = add(add(bytes, key.capacity())?, walk(child, depth + 1)?)?;
                }
            }
            _ => {}
        }
        Ok(bytes)
    }
    walk(value, 0)
}
impl StateBudget {
    pub fn new(limit: usize) -> Self { Self { limit, charged: 0 } }
    pub fn charged(&self) -> usize { self.charged }
    pub fn charge(&mut self, bytes: usize) -> Result<(), FailureKind> {
        self.charged = self.charged.checked_add(bytes)
            .filter(|n| *n <= self.limit).ok_or(FailureKind::BodyTooLarge)?;
        Ok(())
    }
    pub fn retain(&mut self, value: &Value) -> Result<(), FailureKind> {
        self.charge(value_bytes(value)?)
    }
    pub fn copy(&mut self, value: &Value) -> Result<Value, FailureKind> {
        self.retain(value)?;
        Ok(value.clone())
    }
    pub fn parse(&mut self, bytes: &[u8]) -> Result<Value, FailureKind> {
        // Count structural tokens outside strings without allocating. A 4 KiB
        // reserve per structural token dominates Value, BTreeMap/IndexMap
        // nodes and array slack on this pinned 64-bit target. String scratch,
        // decoded strings and number parsing receive four wire lengths.
        let (mut string, mut escape, mut nodes) = (false, false, 1usize);
        for b in bytes {
            if string {
                if escape { escape = false; }
                else if *b == b'\\' { escape = true; }
                else if *b == b'"' { string = false; }
            } else if *b == b'"' { string = true; }
            else if b"{}[],:".contains(b) {
                nodes = nodes.checked_add(1).ok_or(FailureKind::BodyTooLarge)?;
            }
        }
        let bound = nodes.checked_mul(4096)
            .and_then(|n| bytes.len().checked_mul(4).and_then(|s| n.checked_add(s)))
            .ok_or(FailureKind::BodyTooLarge)?;
        self.charge(bound)?;
        serde_json::from_slice(bytes).map_err(|_| FailureKind::UnsupportedResponse)
    }
}
