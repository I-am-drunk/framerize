use super::*;
use serde_json::json;

#[derive(Default)]
struct Wire { bytes: Vec<u8>, boundaries: Vec<(u64, bool, usize)>, ended: usize }
impl Write for Wire {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.bytes.extend_from_slice(bytes); Ok(bytes.len()) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl ResponseWriter for Wire {
    fn begin_event(&mut self, sequence: u64, content: bool) -> io::Result<()> {
        self.boundaries.push((sequence, content, self.bytes.len())); Ok(())
    }
    fn end_event(&mut self) -> io::Result<()> { self.ended += 1; Ok(()) }
}
fn result(output: Vec<Value>) -> AssembledResponses {
    AssembledResponses { id: "upstream".into(), created_at: 1, output, status: "completed".into(),
        incomplete_details: None, usage: json!({"details":{"kept":true}}), service_tier: json!("priority"),
        input_tokens: 2, output_tokens: 3, chunks: 1, metered_chunks: 0 }
}
fn events(bytes: &[u8]) -> Vec<Value> {
    std::str::from_utf8(bytes).unwrap().lines().filter_map(|line| line.strip_prefix("data: "))
        .map(|value| serde_json::from_str(value).unwrap()).collect()
}
#[test]
fn incremental_tools_and_opaque_fields_keep_identity_and_escaping() {
    let items = vec![json!({"type":"function_call","call_id":"call","name":"tool",
        "namespace":"kept","arguments":"\"雪\\\n","extra":{"future":[1,2]}}),
        json!({"type":"custom_tool_call","call_id":"custom","name":"text","input":""}),
        json!({"type":"future_output","payload":{"opaque":"λ"}})];
    let mut encoder = ResponseEncoder::new("model", "logical");
    let mut wire = Wire::default();
    encoder.open(&mut wire).unwrap();
    encoder.push_items(&mut wire, &items, false).unwrap();
    encoder.finish(&mut wire, &result(items.clone())).unwrap();
    let events = events(&wire.bytes);
    assert!(events.iter().enumerate().all(|(i, e)| e["sequence_number"] == i));
    assert_eq!(wire.ended, events.len());
    for (event, (_, content, _)) in events.iter().zip(&wire.boundaries) {
        assert_eq!(*content, !matches!(event["type"].as_str(),
            Some("response.created" | "response.in_progress" | "response.completed")));
    }
    let final_output = &events.last().unwrap()["response"]["output"];
    assert_eq!(final_output[0]["id"], "fc_logical_1");
    assert_eq!(final_output[0]["arguments"], items[0]["arguments"]);
    assert_eq!(final_output[0]["extra"], items[0]["extra"]);
    assert_eq!(final_output[1]["id"], "ctc_logical_2");
    assert_eq!(final_output[2], items[2]);
    assert_eq!(events.last().unwrap()["response"]["usage"]["details"], json!({"kept":true}));
}
#[test]
fn text_sizes_and_rich_parts_preserve_content_without_event_batches() {
    for size in [0, 1, 16383, 16384, 16385, 32768] {
        let text = "x".repeat(size);
        let item = json!({"type":"message","role":"assistant","content":[
            {"type":"output_text","text":text,"annotations":[{"kind":"future"}],"extra":"kept"},
            {"type":"refusal","refusal":"no"}]});
        let mut wire = Wire::default();
        let mut encoder = ResponseEncoder::new("model", "resp_fixed");
        encoder.push_items(&mut wire, &[item.clone()], false).unwrap();
        encoder.finish(&mut wire, &result(vec![item.clone()])).unwrap();
        let events = events(&wire.bytes);
        let output = &events.last().unwrap()["response"]["output"][0];
        assert_eq!(output["content"], item["content"]);
        assert_eq!(output["id"], "msg_fixed_1");
        assert_eq!(wire.ended, events.len());
    }
}
struct Broken { bytes: usize, ended: usize }
impl Write for Broken {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes >= 50 { return Err(io::Error::other("injected write failure")); }
        let amount = bytes.len().min(50 - self.bytes); self.bytes += amount; Ok(amount)
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl ResponseWriter for Broken {
    fn begin_event(&mut self, _: u64, _: bool) -> io::Result<()> { Ok(()) }
    fn end_event(&mut self) -> io::Result<()> { self.ended += 1; Ok(()) }
}
#[test]
fn failed_serialization_never_marks_an_event_complete() {
    let mut writer = Broken { bytes: 0, ended: 0 };
    let mut encoder = ResponseEncoder::new("fixture", "fixture");
    assert!(encoder.open(&mut writer).is_err());
    assert_eq!(writer.ended, 0);
    assert_eq!(writer.bytes, 50);
}
#[test]
fn fixed_failure_uses_publisher_frontier() {
    let mut wire = Wire::default();
    ResponseEncoder::fail_at(&mut wire, 7, "upstream operation task failed").unwrap();
    let values = events(&wire.bytes);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["sequence_number"], 7);
    assert_eq!(values[0]["type"], "response.failed");
    assert!(!String::from_utf8(wire.bytes).unwrap().contains("response.completed"));
}
#[test]
fn borrowed_objects_keep_legacy_sorted_json_bytes() {
    let base = json!({"z":{"b":2,"a":1},"id":"old","call_id":"call","arguments":"x"});
    let view = Overlay(&base, vec![("id", Text("new")), ("arguments", Text(""))]);
    let expected = json!({"z":{"b":2,"a":1},"id":"new","call_id":"call","arguments":""});
    assert_eq!(serde_json::to_string(&view).unwrap(), expected.to_string());
    let mut wire = Wire::default();
    ResponseEncoder::new("fixture", "logical").open(&mut wire).unwrap();
    for frame in std::str::from_utf8(&wire.bytes).unwrap().split("\n\n").filter(|s| !s.is_empty()) {
        let (header, data) = frame.split_once("\ndata: ").unwrap();
        let value: Value = serde_json::from_str(data).unwrap();
        assert_eq!(frame, format!("{header}\ndata: {value}"));
    }
}

#[test]
fn incremental_wire_matches_frozen_tool_and_opaque_event_contract() {
    let tool = json!({"type":"function_call","call_id":"call","name":"tool","arguments":"雪\n"});
    let opaque = json!({"type":"future_output","opaque":{"z":1,"a":2}});
    let mut wire = Wire::default();
    let mut encoder = ResponseEncoder::new("model", "logical");
    encoder.push_items(&mut wire, &[tool.clone(), opaque.clone()], false).unwrap();
    encoder.finish(&mut wire, &result(vec![tool.clone(), opaque.clone()])).unwrap();
    let mut added = tool.clone();
    added["id"] = json!("fc_logical_1"); added["arguments"] = json!(""); added["status"] = json!("in_progress");
    let mut done = tool; done["id"] = json!("fc_logical_1");
    let expected = [
        ("response.output_item.added", json!({"output_index":0,"item":added})),
        ("response.function_call_arguments.delta", json!({"output_index":0,"item_id":"fc_logical_1","delta":"雪\n"})),
        ("response.function_call_arguments.done", json!({"output_index":0,"item_id":"fc_logical_1",
            "arguments":"雪\n","call_id":"call","name":"tool"})),
        ("response.output_item.done", json!({"output_index":0,"item":done.clone()})),
        ("response.output_item.added", json!({"output_index":1,"item":opaque.clone()})),
        ("response.output_item.done", json!({"output_index":1,"item":opaque.clone()})),
        ("response.completed", json!({"response":{"id":"resp_logical","object":"response",
            "created_at":1,"status":"completed","model":"model","output":[done,opaque],
            "service_tier":"priority","usage":{"details":{"kept":true},"input_tokens":2,
                "output_tokens":3,"total_tokens":5}}})),
    ];
    let golden: String = expected.into_iter().enumerate().map(|(sequence, (kind, mut event))| {
        event["type"] = json!(kind); event["sequence_number"] = json!(sequence);
        format!("event: {kind}\ndata: {event}\n\n")
    }).collect();
    assert_eq!(String::from_utf8(wire.bytes).unwrap(), golden);
}
