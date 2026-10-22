//! Borrowed, single-frame serialization into an already admitted allocation.
use super::FailureKind;
use serde::{Serialize, Serializer, ser::{SerializeMap, SerializeSeq}};
use serde_json::Value;
use std::io::{self, Write};

pub enum Field<'a> {
    Value(&'a Value), Values(&'a [Value]), Str(&'a str), Int(u64),
    Null, Object(&'a [(&'a str, Field<'a>)]), Array(&'a [Field<'a>]),
    Encoded(&'a Value),
    Overlay(&'a Value, &'a [(&'a str, Field<'a>)]),
}
pub struct Object<'a>(pub &'a [(&'a str, Field<'a>)], pub Option<&'a str>, pub Option<u64>);
impl Serialize for Object<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let ty = self.1.filter(|_| !self.0.iter().any(|(key, _)| *key == "type"));
        let mut map = serializer.serialize_map(Some(self.0.len()
            + usize::from(ty.is_some()) + usize::from(self.2.is_some())))?;
        if let Some(ty) = ty { map.serialize_entry("type", ty)?; }
        if let Some(sequence) = self.2 { map.serialize_entry("sequence_number", &sequence)?; }
        for (key, value) in self.0 { map.serialize_entry(key, value)?; }
        map.end()
    }
}
impl Serialize for Field<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Value(v) => v.serialize(serializer),
            Self::Values(v) => v.serialize(serializer),
            Self::Str(v) => serializer.serialize_str(v),
            Self::Int(v) => serializer.serialize_u64(*v),
            Self::Null => serializer.serialize_none(),
            Self::Encoded(v) => serializer.collect_str(v),
            Self::Overlay(v, extra) => {
                let mut map = serializer.serialize_map(None)?;
                if let Some(original) = v.as_object() {
                    for (key, value) in original {
                        if !extra.iter().any(|(name, _)| *name == key) {
                            map.serialize_entry(key, value)?;
                        }
                    }
                }
                for (key, value) in *extra { map.serialize_entry(key, value)?; }
                map.end()
            }
            Self::Object(v) => Object(v, None, None).serialize(serializer),
            Self::Array(v) => {
                let mut seq = serializer.serialize_seq(Some(v.len()))?;
                for item in *v { seq.serialize_element(item)?; }
                seq.end()
            }
        }
    }
}

struct Fixed<'a> { bytes: &'a mut [u8], len: usize }
impl Write for Fixed<'_> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let end = self.len.checked_add(input.len())
            .filter(|n| *n <= self.bytes.len())
            .ok_or_else(|| io::Error::other("frame budget"))?;
        self.bytes[self.len..end].copy_from_slice(input);
        self.len = end;
        Ok(input.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

pub fn frame(bytes: &mut [u8], event: Option<&str>, fields: &[(&str, Field<'_>)], sequence: Option<u64>)
    -> Result<usize, FailureKind> {
    let mut out = Fixed { bytes, len: 0 };
    let write = (|| -> io::Result<()> {
        if let Some(event) = event {
            out.write_all(b"event: ")?;
            out.write_all(event.as_bytes())?;
            out.write_all(b"\n")?;
        }
        out.write_all(b"data: ")?;
        serde_json::to_writer(&mut out, &Object(fields, event, sequence)).map_err(io::Error::other)?;
        out.write_all(b"\n\n")
    })();
    write.map_err(|_| FailureKind::BodyTooLarge)?;
    Ok(out.len)
}

pub fn done(bytes: &mut [u8]) -> Result<usize, FailureKind> {
    let frame = b"data: [DONE]\n\n";
    if frame.len() > bytes.len() { return Err(FailureKind::BodyTooLarge); }
    bytes[..frame.len()].copy_from_slice(frame);
    Ok(frame.len())
}
