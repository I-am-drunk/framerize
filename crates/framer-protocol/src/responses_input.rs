use serde_json::{json, Value};
use crate::convert::ChainEvent;

pub fn merge_items(acc: &mut Vec<Value>, new: Vec<Value>) {
    for it in new {
        let is_msg = it["type"].as_str() == Some("message");
        if is_msg {
            if let Some(last) = acc.last_mut() {
                if last["type"].as_str() == Some("message") {
                    crate::convert::merge_response_message(last, &it);
                    continue;
                }
            }
        }
        acc.push(it);
    }
}

pub fn record_chunk_events(items: &[Value], continues_text: bool, events: Option<&(dyn Fn(ChainEvent) + Send + Sync)>) {
    if let Some(cb) = events {
        cb(ChainEvent::ResponsesItems {
            items: items.to_vec(),
            continues_text,
        });
    }
}

pub fn prepare_input(client_payload: &Value) -> Vec<Value> {
    prepare_input_controlled(client_payload, None)
}

pub(crate) fn prepare_input_controlled(client_payload: &Value,
    control: Option<&dyn crate::forward::ChainControl>) -> Vec<Value> {
    // Normalize the client's input to an item array (a bare string prompt is
    // legal Responses input).
    let mut input_items: Vec<Value> = match &client_payload["input"] {
        Value::String(s) => vec![json!({
            "type": "message", "role": "user",
            "content": [{"type": "input_text", "text": s}],
        })],
        Value::Array(a) => a.clone(),
        _ => Vec::new(),
    };

    // Namespace backfill (heals sessions poisoned before the verbatim-streaming
    // fix): the upstream rejects re-sent function_calls missing `namespace`
    // when the tool lives outside the default namespace ("Missing namespace for
    // function_call 'spawn_agent'", 2026-10-06). Recover the namespace from the
    // request's own tool declarations: a `namespace` field on the tool, a
    // `type:"namespace"` container holding it, or a dotted `ns.name`.
    let ns_of = {
        let mut map: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
        // A bare name must identify one namespace. Default-namespace tools
        // and conflicting declarations make inference unsafe in either order.
        fn record(map: &mut std::collections::HashMap<String, Option<String>>,
            name: &str, namespace: Option<&str>) {
            map.entry(name.to_string()).and_modify(|known| {
                if known.as_deref() != namespace { *known = None; }
            }).or_insert_with(|| namespace.map(str::to_string));
        }
        if let Some(tools) = client_payload["tools"].as_array() {
            for t in tools {
                let ty = t["type"].as_str().unwrap_or_default();
                if ty == "namespace" {
                    if let Some(ns) = t["name"].as_str() {
                        for inner in t["tools"].as_array().into_iter().flatten() {
                            if let Some(n) = inner["name"].as_str() {
                                record(&mut map, n, Some(ns));
                            }
                        }
                    }
                } else if let Some(n) = t["name"].as_str() {
                    if let Some(ns) = t["namespace"].as_str() {
                        record(&mut map, n, Some(ns));
                    } else if let Some((ns, _)) = n.split_once('.') {
                        record(&mut map, n.rsplit('.').next().unwrap_or(n), Some(ns));
                    } else {
                        record(&mut map, n, None);
                    }
                }
            }
        }
        map
    };
    if !ns_of.is_empty() {
        let mut healed = 0u32;
        for it in input_items.iter_mut() {
            let ty = it["type"].as_str().unwrap_or_default();
            if (ty == "function_call" || ty == "custom_tool_call") && it.get("namespace").is_none() {
                if let Some(ns) = it["name"].as_str().and_then(|n| ns_of.get(n))
                    .and_then(Option::as_ref) {
                    it["namespace"] = json!(ns);
                    healed += 1;
                }
            }
        }
        if healed > 0 {
            eprintln!("[nativechain] namespace backfill: healed {healed} call item(s) from tool declarations");
            if let Some(control) = control {
                control.diag(crate::forward::PolicyDiag::NamespaceBackfill { count: healed.into() });
            }
        }
    }

    input_items
}

pub fn prepare_payload(client_payload: &Value, input: &[Value], chunk_budget: u32) -> Value {
    let mut p = client_payload.clone();
    p["model"] = json!(crate::convert::to_openrouter_id(
        client_payload["model"].as_str().unwrap_or_default()
    ));
    p["stream"] = json!(false);
    p["max_output_tokens"] = json!(chunk_budget.max(16)); // API floor
    p["input"] = json!(input);
    for k in ["store", "previous_response_id", "background"] {
        if let Some(obj) = p.as_object_mut() {
            obj.remove(k);
        }
    }
    p
}

/// Preparation for a selected free Responses worker invocation. The kernel
/// sends the original payload, so retries and repicks never append this twice.
pub fn prepare_worker_payload(mut payload: Value) -> Value {
    if payload["tools"].as_array().is_some_and(|tools| !tools.is_empty()) {
        let note = "When using tools to write or edit files, split large writes: at most ~40 lines per tool call, then append the next section with a separate call, and repeat until done.";
        payload["instructions"] = match payload["instructions"].as_str() {
            Some(existing) if !existing.is_empty() => Value::String(format!("{existing}\n\n{note}")),
            _ => Value::String(note.into()),
        };
    }
    payload
}
