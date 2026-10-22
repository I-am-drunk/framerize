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
    assert_eq!(take_reply_result(json!({"ok": true})), Ok(Value::Null));
    assert_eq!(take_reply_result(json!({"ok": true, "result": false})), Ok(json!(false)));
    assert_eq!(
        take_reply_result(json!({"ok": false, "error": "fixture failure"})),
        Err("fixture failure".to_owned()),
    );
    for value in [json!({}), json!(null), json!({"ok": true, "result": null})] {
        let expected = if value["ok"] == true {
            Ok(Value::Null)
        } else {
            Err("sidecar error".to_owned())
        };
        assert_eq!(take_reply_result(value), expected);
    }
}
