//! Decode fields directly; internally tagged derives buffer every payload byte
//! as a generic Content value before constructing its typed Segment.
use super::*;
use serde::de::{self, value::MapAccessDeserializer, MapAccess, SeqAccess, Visitor};
use std::fmt;

struct Field<T>(Option<T>);
impl<T> Default for Field<T> {
    fn default() -> Self { Self(None) }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Field<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        T::deserialize(d).map(|value| Self(Some(value)))
    }
}
impl<T> Field<T> {
    fn take<E: de::Error>(&mut self, name: &'static str) -> Result<T, E> {
        self.0.take().ok_or_else(|| E::missing_field(name))
    }
}

// The same field name has a string enum in one message and an object in another.
// Only these small enums retain Serde buffering, never the transport payload.
#[derive(Deserialize)]
#[serde(untagged)]
enum Shape<S, M> { Text(S), Map(M) }
impl<S, M> Shape<S, M> {
    fn text<E: de::Error>(self) -> Result<S, E> {
        match self { Self::Text(value) => Ok(value), _ => Err(E::custom("expected string enum")) }
    }
    fn map<E: de::Error>(self) -> Result<M, E> {
        match self { Self::Map(value) => Ok(value), _ => Err(E::custom("expected object enum")) }
    }
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Hello, Ready, Start, Invoke, Head, Forward, ForwardReply, ForwardError,
    Saturated, SaturatedAck, Data, Diag, Result, Finish, Fail, Cancel, Done, Credit,
}

macro_rules! fields {
    ($($name:ident: $ty:ty),+ $(,)?) => {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields { kind: Kind, $(#[serde(default)] $name: Field<$ty>,)+ }
        impl Fields {
            fn finish<E: de::Error>(&self) -> Result<(), E> {
                $(if self.$name.0.is_some() {
                    return Err(E::custom(concat!("unexpected field: ", stringify!($name))));
                })+
                Ok(())
            }
        }
    };
}
fields! {
    wire: u16, build: Text, routes: Vec<Route>, generation: u64,
    op: u64, logical_id: Text, model: Text, mode: Mode, output: Capacity,
    n: u32, category: Text, limits: Limits, request: Part,
    status: u16, content_type: Text, fwd: u64, dialect: Dialect,
    step: Text, step_index: u32, payload: Part, queue_wait_ms: u64,
    body: Part, class: ErrorClass, cooldown_seconds: Option<u64>,
    event: Shape<DataKind, Diagnostic>, seq: u64, bytes: Segment, last: bool,
    outcome: Shape<DoneOutcome, Outcome>, diag_dropped: u32,
    error: PublicFailure, returned: Capacity, accepted_event_seq: u64,
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(MessageVisitor)
    }
}
struct MessageVisitor;
impl<'de> Visitor<'de> for MessageVisitor {
    type Value = Message;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a tagged worker message")
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Message, A::Error> {
        Fields::deserialize(MapAccessDeserializer::new(map))?.into_message()
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Message, A::Error> {
        // Retain the positional representation accepted by the prior derive.
        macro_rules! take { ($name:ident) => {
            seq.next_element()?.ok_or_else(|| <A::Error as de::Error>::missing_field(stringify!($name)))?
        }; }
        macro_rules! message { ($kind:ident { $($field:ident),* }) => {
            Message::$kind { $($field: take!($field)),* }
        }; }
        let kind: Kind = take!(kind);
        let message = match kind {
            Kind::Hello => message!(Hello { wire, build, routes }),
            Kind::Ready => message!(Ready { generation }),
            Kind::Start => message!(Start { op, logical_id, model, mode, output }),
            Kind::Invoke => message!(Invoke { op, n, category, limits, request }),
            Kind::Head => message!(Head { op, status, content_type }),
            Kind::Forward => message!(Forward { op, n, fwd, dialect, step, step_index, payload }),
            Kind::ForwardReply => message!(ForwardReply { op, n, fwd, status, content_type, queue_wait_ms, body }),
            Kind::ForwardError => Message::ForwardError {
                op: take!(op), n: take!(n), fwd: take!(fwd), class: take!(class),
                cooldown_seconds: take!(cooldown_seconds),
            },
            Kind::Saturated => message!(Saturated { op, n, fwd }),
            Kind::SaturatedAck => message!(SaturatedAck { op, n, fwd }),
            Kind::Data => message!(Data { op, event, seq, bytes, last }),
            Kind::Diag => message!(Diag { op, n, event }),
            Kind::Result => message!(Result { op, n, outcome, diag_dropped }),
            Kind::Finish => message!(Finish { op, n }),
            Kind::Fail => message!(Fail { op, n, error }),
            Kind::Cancel => message!(Cancel { op }),
            Kind::Done => message!(Done { op, outcome, diag_dropped }),
            Kind::Credit => message!(Credit { op, returned, accepted_event_seq }),
        };
        if seq.next_element::<de::IgnoredAny>()?.is_some() {
            return Err(de::Error::custom("unexpected trailing message field"));
        }
        Ok(message)
    }
}

impl Fields {
    fn into_message<E: de::Error>(self) -> Result<Message, E> {
        let mut fields = self;
        macro_rules! take { ($field:ident) => { fields.$field.take::<E>(stringify!($field))? }; }
        macro_rules! message {
            ($kind:ident { $($field:ident),* }) => {
                Message::$kind { $($field: take!($field)),* }
            };
        }
        let message = match fields.kind {
            Kind::Hello => message!(Hello { wire, build, routes }),
            Kind::Ready => message!(Ready { generation }),
            Kind::Start => message!(Start { op, logical_id, model, mode, output }),
            Kind::Invoke => message!(Invoke { op, n, category, limits, request }),
            Kind::Head => message!(Head { op, status, content_type }),
            Kind::Forward => message!(Forward { op, n, fwd, dialect, step, step_index, payload }),
            Kind::ForwardReply => message!(ForwardReply { op, n, fwd, status, content_type, queue_wait_ms, body }),
            Kind::ForwardError => Message::ForwardError {
                op: take!(op), n: take!(n), fwd: take!(fwd), class: take!(class),
                cooldown_seconds: fields.cooldown_seconds.0.take().unwrap_or(None),
            },
            Kind::Saturated => message!(Saturated { op, n, fwd }),
            Kind::SaturatedAck => message!(SaturatedAck { op, n, fwd }),
            Kind::Data => Message::Data {
                op: take!(op), event: take!(event).text::<E>()?, seq: take!(seq),
                bytes: take!(bytes), last: take!(last),
            },
            Kind::Diag => Message::Diag {
                op: take!(op), n: take!(n), event: take!(event).map::<E>()?,
            },
            Kind::Result => Message::Result {
                op: take!(op), n: take!(n), outcome: take!(outcome).map::<E>()?,
                diag_dropped: take!(diag_dropped),
            },
            Kind::Finish => message!(Finish { op, n }),
            Kind::Fail => message!(Fail { op, n, error }),
            Kind::Cancel => message!(Cancel { op }),
            Kind::Done => Message::Done {
                op: take!(op), outcome: take!(outcome).text::<E>()?,
                diag_dropped: take!(diag_dropped),
            },
            Kind::Credit => message!(Credit { op, returned, accepted_event_seq }),
        };
        fields.finish::<E>()?;
        Ok(message)
    }
}

#[cfg(test)]
#[path = "message_decode_tests.rs"]
mod tests;
