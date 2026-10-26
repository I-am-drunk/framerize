use super::*;

fn call(kind: &str, name: &str) -> Value {
    let mut item = json!({"type":kind,"name":name,"id":"item-fixture",
        "call_id":"call-fixture","status":"completed"});
    if kind == "function_call" { item["arguments"] = json!("{}"); }
    else { item["input"] = json!("fixture text"); }
    item
}

#[test]
fn overlapping_namespaces_never_choose_the_last_declaration() {
    for kind in ["function_call", "custom_tool_call"] {
        for names in [["files", "database", "files"], ["database", "files", "database"]] {
            let original = call(kind, "read");
            let tools: Vec<Value> = names.iter().map(|name| namespace(name, "read", kind)).collect();
            let payload = json!({"input":[original],"tools":tools});
            assert_eq!(prepare_input(&payload), vec![original]);
        }
    }
}

fn namespace(name: &str, tool: &str, call_kind: &str) -> Value {
    let kind = if call_kind == "custom_tool_call" { "custom" } else { "function" };
    json!({"type":"namespace","name":name,"tools":[{"type":kind,"name":tool}]})
}

#[test]
fn unique_namespace_is_recovered_without_changing_call_fields() {
    for kind in ["function_call", "custom_tool_call"] {
        let original = call(kind, "read");
        let payload = json!({"input":[original],"tools":[namespace("files", "read", kind)]});
        let mut expected = original;
        expected["namespace"] = json!("files");
        assert_eq!(prepare_input(&payload), vec![expected]);
        assert!(payload["input"][0].get("namespace").is_none());
    }
}

#[test]
fn a_default_namespace_tool_cannot_be_claimed_by_a_named_namespace() {
    let original = call("function_call", "read");
    let default = json!({"type":"function","name":"read"});
    for named in [namespace("files", "read", "function_call"),
        json!({"type":"function","name":"read","namespace":"files"}),
        json!({"type":"function","name":"files.read"})] {
        for tools in [vec![default.clone(), named.clone()], vec![named, default.clone()]] {
            let payload = json!({"input":[original],"tools":tools});
            assert_eq!(prepare_input(&payload), vec![original.clone()]);
        }
    }
}

#[test]
fn agreeing_declaration_styles_keep_unambiguous_repair() {
    let original = call("function_call", "read");
    let payload = json!({"input":[original],"tools":[
        namespace("files", "read", "function_call"),
        {"type":"function","name":"read","namespace":"files"},
        {"type":"function","name":"files.read"}]});
    let mut expected = original;
    expected["namespace"] = json!("files");
    assert_eq!(prepare_input(&payload), vec![expected]);
}

#[test]
fn explicit_namespaces_and_unrelated_items_remain_verbatim() {
    let mut explicit = call("custom_tool_call", "read");
    explicit["namespace"] = json!("original");
    let input = vec![explicit, call("function_call", "unknown"),
        json!({"type":"function_call_output","call_id":"call-fixture","name":"read",
            "output":"result"}),
        json!({"type":"message","role":"user","content":"continue"})];
    let payload = json!({"input":input,"tools":[namespace("files", "read", "custom_tool_call")]});
    assert_eq!(prepare_input(&payload), input);
}
