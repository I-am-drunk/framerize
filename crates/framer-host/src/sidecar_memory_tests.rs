use super::take_reply_result;
use serde_json::{json, Value};

#[test]
fn result_handoff_moves_large_strings_without_copying() {
    let reply = json!({
        "id": 7,
        "ok": true,
        "result": {
            "body": "synthetic text ".repeat(16384),
            "status": 200,
            "nested": ["one", "two"],
        },
    });
    let body = reply["result"]["body"].as_str().unwrap();
    let pointer = body.as_ptr();
    let length = body.len();
    let array_pointer = reply["result"]["nested"].as_array().unwrap().as_ptr();
    let result = take_reply_result(reply).unwrap();
    let moved = result["body"].as_str().unwrap();
    assert_eq!(moved.as_ptr(), pointer);
    assert_eq!(moved.len(), length);
    assert_eq!(result["nested"].as_array().unwrap().as_ptr(), array_pointer);
    assert_eq!(result["status"], 200);
    assert_eq!(result["nested"], json!(["one", "two"]));
}

#[test]
fn result_handoff_preserves_missing_results_and_error_messages() {
    for (case, reply, expected) in [
        ("missing result", json!({"ok": true}), Ok(Value::Null)),
        ("false result", json!({"ok": true, "result": false}), Ok(json!(false))),
        ("explicit error", json!({"ok": false, "error": "fixture failure"}), Err("fixture failure")),
        ("missing status", json!({}), Err("sidecar error")),
        ("non-object reply", json!(null), Err("sidecar error")),
        ("null result", json!({"ok": true, "result": null}), Ok(Value::Null)),
    ] {
        assert_eq!(take_reply_result(reply), expected.map_err(str::to_owned), "{case}");
    }
}
