use super::*;
use serde_json::Value;

const CASES: &[&str] = &[
    r#"{"kind":"hello","wire":1,"build":"test","routes":["free_responses"]}"#,
    r#"{"kind":"ready","generation":7}"#,
    r#"{"kind":"start","op":1,"logical_id":"id","model":"model","mode":"sse","output":{"wire_bytes":100,"slots":2,"downstream_bytes":10}}"#,
    r#"{"kind":"invoke","op":1,"n":0,"category":"translation","limits":{"chunk_tokens":600,"max_chunks":4,"max_total_tokens":null,"metered_fallback":false,"continue_prompt":"continue"},"request":{"offset":0,"bytes":[0,255],"last":true}}"#,
    r#"{"kind":"head","op":1,"status":200,"content_type":"text/event-stream"}"#,
    r#"{"kind":"forward","op":1,"n":0,"fwd":1,"dialect":"open_router_responses","step":"translate","step_index":0,"payload":{"offset":0,"bytes":[0,255],"last":true}}"#,
    r#"{"kind":"forward_reply","op":1,"n":0,"fwd":1,"status":200,"content_type":"application/json","queue_wait_ms":7,"body":{"offset":0,"bytes":[0,255],"last":true}}"#,
    r#"{"kind":"forward_error","op":1,"n":0,"fwd":1,"class":"retryable429","cooldown_seconds":null}"#,
    r#"{"kind":"saturated","op":1,"n":0,"fwd":1}"#,
    r#"{"kind":"saturated_ack","op":1,"n":0,"fwd":1}"#,
    r#"{"kind":"data","op":1,"event":"content","seq":0,"bytes":[0,255],"last":true}"#,
    r#"{"kind":"diag","op":1,"n":0,"event":{"kind":"backoff","chunk":0,"attempt":1,"wait_ms":0}}"#,
    r#"{"kind":"result","op":1,"n":0,"outcome":{"kind":"control"},"diag_dropped":0}"#,
    r#"{"kind":"finish","op":1,"n":0}"#,
    r#"{"kind":"fail","op":1,"n":0,"error":{"status":429,"message":"busy","type":"api_error","code":"busy"}}"#,
    r#"{"kind":"cancel","op":1}"#,
    r#"{"kind":"done","op":1,"outcome":"completed","diag_dropped":0}"#,
    r#"{"kind":"credit","op":1,"returned":{"wire_bytes":100,"slots":2,"downstream_bytes":10},"accepted_event_seq":1}"#,
];

struct Ordered(Vec<(String, Value)>);
impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct OrderedVisitor;
        impl<'de> Visitor<'de> for OrderedVisitor {
            type Value = Ordered;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Ordered, A::Error> {
                let mut fields = Vec::new();
                while let Some(entry) = map.next_entry()? { fields.push(entry); }
                Ok(Ordered(fields))
            }
        }
        d.deserialize_map(OrderedVisitor)
    }
}
fn object(fields: &[(String, Value)]) -> String {
    let fields: Vec<_> = fields.iter().map(|(k, v)| {
        format!("{}:{}", serde_json::to_string(k).unwrap(), v)
    }).collect();
    format!("{{{}}}", fields.join(","))
}

#[test]
fn all_message_variants_preserve_wire_bytes_and_arbitrary_field_order() {
    for json in CASES {
        let expected: Message = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&expected).unwrap(), *json);
        let mut fields = serde_json::from_str::<Ordered>(json).unwrap().0;
        for _ in 0..fields.len() {
            fields.rotate_left(1);
            let reordered = object(&fields);
            assert_eq!(serde_json::from_str::<Message>(&reordered).unwrap(), expected);
            assert_eq!(serde_json::from_reader::<_, Message>(reordered.as_bytes()).unwrap(), expected);
        }
        fields.reverse();
        assert_eq!(serde_json::from_str::<Message>(&object(&fields)).unwrap(), expected);
        let value: Value = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::from_value::<Message>(value).unwrap(), expected);
    }
}

#[test]
fn positional_messages_preserve_the_prior_serde_representation() {
    for json in CASES {
        let fields = serde_json::from_str::<Ordered>(json).unwrap().0;
        let positional: Vec<_> = fields.into_iter().map(|(_, value)| value).collect();
        let expected: Message = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::from_value::<Message>(Value::Array(positional)).unwrap(), expected);
    }
}

#[test]
fn required_and_foreign_fields_remain_strict_for_every_message_kind() {
    let all: Vec<Value> = CASES.iter().map(|s| serde_json::from_str(s).unwrap()).collect();
    for value in &all {
        let object = value.as_object().unwrap();
        for name in object.keys() {
            if name == "cooldown_seconds" { continue; }
            let mut missing = object.clone();
            missing.remove(name);
            assert!(serde_json::from_value::<Message>(Value::Object(missing)).is_err(), "missing {name}");
            let mut null = object.clone();
            null.insert(name.clone(), Value::Null);
            assert!(serde_json::from_value::<Message>(Value::Object(null)).is_err(), "null {name}");
        }
        for other in &all {
            for (name, data) in other.as_object().unwrap() {
                if object.contains_key(name) { continue; }
                let mut foreign = object.clone();
                foreign.insert(name.clone(), data.clone());
                assert!(serde_json::from_value::<Message>(Value::Object(foreign)).is_err(), "foreign {name}");
            }
        }
    }
}

#[test]
fn every_duplicate_field_and_unknown_field_remains_rejected() {
    for json in CASES {
        let fields = serde_json::from_str::<Ordered>(json).unwrap().0;
        for extra in fields.iter().cloned().chain([("extra".into(), Value::Null)]) {
            for first in [false, true] {
                let mut duplicate = fields.clone();
                if first { duplicate.insert(0, extra.clone()); }
                else { duplicate.push(extra.clone()); }
                assert!(serde_json::from_str::<Message>(&object(&duplicate)).is_err());
            }
        }
    }
}

#[test]
fn optional_cooldown_accepts_only_its_own_message_and_preserves_absence() {
    let json = r#"{"kind":"forward_error","op":1,"n":0,"fwd":1,"class":"other"}"#;
    let mut value: Value = serde_json::from_str(json).unwrap();
    let absent: Message = serde_json::from_value(value.clone()).unwrap();
    value["cooldown_seconds"] = Value::Null;
    assert_eq!(serde_json::from_value::<Message>(value.clone()).unwrap(), absent);
    value["cooldown_seconds"] = Value::from(u64::MAX);
    assert!(matches!(serde_json::from_value::<Message>(value).unwrap(),
        Message::ForwardError { cooldown_seconds: Some(u64::MAX), .. }));
    assert!(serde_json::from_str::<Message>(r#"{"kind":"cancel","op":1,"cooldown_seconds":null}"#).is_err());
}

#[test]
fn payload_limits_and_nested_positional_variants_remain_compatible() {
    for size in [0, 1, crate::SEGMENT_LIMIT] {
        let bytes = serde_json::to_string(&vec![255; size]).unwrap();
        let json = format!(r#"{{"bytes":{bytes},"op":1,"event":"content","seq":0,"last":true,"kind":"data"}}"#);
        let message: Message = serde_json::from_str(&json).unwrap();
        assert!(matches!(message, Message::Data { bytes, .. } if bytes.as_slice() == vec![255; size]));
    }
    for bytes in [serde_json::to_string(&vec![0; crate::SEGMENT_LIMIT + 1]).unwrap(),
        "[256]".into(), "[-1]".into(), "[null]".into(), "null".into()] {
        let json = format!(r#"{{"bytes":{bytes},"op":1,"event":"content","seq":0,"last":true,"kind":"data"}}"#);
        assert!(serde_json::from_str::<Message>(&json).is_err());
    }
    let diag = r#"{"kind":"diag","op":1,"n":0,"event":["backoff",0,1,0]}"#;
    assert!(matches!(serde_json::from_str::<Message>(diag).unwrap(),
        Message::Diag { event: Diagnostic::Backoff { .. }, .. }));
    let result = r#"{"kind":"result","op":1,"n":0,"outcome":["local","cancelled"],"diag_dropped":0}"#;
    assert!(matches!(serde_json::from_str::<Message>(result).unwrap(),
        Message::Result { outcome: Outcome::Local { failure: LocalFailure::Cancelled }, .. }));
}

#[test]
fn positional_messages_reject_extra_and_missing_required_values() {
    for json in CASES {
        let fields = serde_json::from_str::<Ordered>(json).unwrap().0;
        let positional: Vec<_> = fields.into_iter().map(|(_, value)| value).collect();
        let mut extra = positional.clone();
        extra.push(Value::Null);
        assert!(serde_json::from_value::<Message>(Value::Array(extra)).is_err());
        let mut missing = positional.clone();
        missing.pop();
        let result = serde_json::from_value::<Message>(Value::Array(missing));
        if positional[0] == "forward_error" { assert!(result.is_ok()); }
        else { assert!(result.is_err()); }
        let mut unknown = positional;
        unknown[0] = Value::from("unknown");
        assert!(serde_json::from_value::<Message>(Value::Array(unknown)).is_err());
    }
}
