use super::*;

#[test]
fn checkpoint_storage_grows_linearly_with_completed_chunks() {
    let sizes: Vec<i64> = [32, 64].iter().map(|count| {
        let store = HistoryStore::default();
        let mut trace = Trace::default();
        let mut content = Vec::new();
        trace.append(&store, &mut content, &[thought("growth-checkpoint")]).unwrap();
        for index in 1..*count {
            trace.continuing(false);
            trace.append(&store, &mut content, &[text(&format!(" chunk {index}"))]).unwrap();
        }
        let mut body = request(content);
        store.restore(&mut body).unwrap();
        assert_eq!(body["messages"].as_array().unwrap().len(), 2 * count + 1);
        assert_eq!(body["messages"][2 * count - 1]["content"][0]["text"],
            format!(" chunk {}", count - 1));
        let bytes = store.0.lock().unwrap().query_row(
            "SELECT sum(length(fingerprint)+length(layout)) FROM checkpoints", [], |r| r.get(0)).unwrap();
        bytes
    }).collect();
    eprintln!("checkpoint storage bytes at 32 and 64 chunks: {sizes:?}");
    assert!(sizes[1] < sizes[0] * 5 / 2, "checkpoint storage sizes: {sizes:?}");
}

fn thought(id: &str) -> Value {
    json!({"type":"thinking","thinking":"fixture thought","signature":id})
}

#[test]
fn linked_history_accepts_an_equivalent_legacy_layout_without_replacing_it() {
    let store = HistoryStore::default();
    let chunks = [vec![thought("legacy")], vec![text("one")], vec![text("two")]];
    let content = record(&store, &chunks);
    let key = content_hash(&content);
    let legacy = store.load(&key).unwrap().unwrap();
    store.0.lock().unwrap().execute("UPDATE checkpoints SET layout=? WHERE fingerprint=?",
        params![legacy.to_string(), key]).unwrap();
    assert_eq!(record(&store, &chunks), content);
    let mut body = request(content);
    store.restore(&mut body).unwrap();
    assert_eq!(body["messages"][5]["content"], json!(chunks[2]));
    let raw: String = store.0.lock().unwrap().query_row(
        "SELECT layout FROM checkpoints WHERE fingerprint=?", [key], |r| r.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&raw).unwrap()["version"], 1);
}
fn text(value: &str) -> Value { json!({"type":"text","text":value}) }
fn request(blocks: Vec<Value>) -> Value {
    json!({"model":"claude-opus-5","messages":[
        {"role":"user","content":"original request"},
        {"role":"assistant","content":blocks},
        {"role":"user","content":"follow up"}]})
}
fn record(store: &HistoryStore, chunks: &[Vec<Value>]) -> Vec<Value> {
    let mut trace = Trace::default();
    let mut merged = Vec::new();
    for chunk in chunks {
        trace.append(store, &mut merged, chunk).unwrap();
        trace.continuing(false);
    }
    merged
}

#[test]
fn restores_original_signed_turns_and_unicode_text_boundaries() {
    let store = HistoryStore::default();
    let chunks = vec![vec![thought("fixture-one"), text("alpha é")],
        vec![text("β"), thought("fixture-two"), text("done")]];
    let mut body = request(record(&store, &chunks));
    store.restore(&mut body).unwrap();
    assert_eq!(body["messages"][1]["content"], json!(chunks[0]));
    assert_eq!(body["messages"][2]["content"], super::super::messages::checkpoint::CONTINUE);
    assert_eq!(body["messages"][3]["content"], json!(chunks[1]));
    assert_eq!(body["messages"][4]["content"], "follow up");
}

#[test]
fn restart_preserves_layout_without_storing_messages_or_signatures() {
    let path = std::env::temp_dir().join(format!("framerize-history-{}", uuid::Uuid::new_v4()));
    let store = HistoryStore::open(&path).unwrap();
    let mut body = request(record(&store, &[vec![thought("private-signature")],
        vec![text("private-response")], vec![text(" private-continuation")]]));
    let data: String = store.0.lock().unwrap().query_row(
        "SELECT group_concat(fingerprint || layout) FROM checkpoints", [], |r| r.get(0)).unwrap();
    assert!(!data.contains("private-signature") && !data.contains("private-response"));
    assert!(!data.contains("fixture thought") && !data.contains("original request"));
    assert!(!data.contains("private-continuation"));
    drop(store);
    HistoryStore::open(&path).unwrap().restore(&mut body).unwrap();
    assert_eq!(body["messages"].as_array().unwrap().len(), 7);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn later_chunks_and_other_branches_keep_earlier_published_history() {
    let store = HistoryStore::default();
    let first = vec![thought("same-first-signature"), text("first")];
    let mut trace = Trace::default();
    let mut combined = Vec::new();
    trace.append(&store, &mut combined, &first).unwrap();
    trace.continuing(false);
    trace.append(&store, &mut combined, &[text(" second")]).unwrap();
    let mut partial = request(combined.clone());
    trace.append(&store, &mut combined, &[text(" third")]).unwrap();
    let mut other = request(record(&store, &[first, vec![text(" alternative")]]));
    store.restore(&mut partial).unwrap();
    store.restore(&mut other).unwrap();
    assert_eq!(partial["messages"][3]["content"][0]["text"], " second");
    assert_eq!(other["messages"][3]["content"][0]["text"], " alternative");
}

#[test]
fn client_cache_markers_do_not_invalidate_checkpoint_history() {
    let store = HistoryStore::default();
    let mut blocks = record(&store, &[vec![thought("signed"), text("one")], vec![text("two")]]);
    blocks[0]["cache_control"] = json!({"type":"ephemeral"});
    blocks[1]["cache_control"] = json!({"type":"ephemeral"});
    let mut body = request(blocks);
    store.restore(&mut body).unwrap();
    assert_eq!(body["messages"][1]["content"][1], text("one"));
    assert_eq!(body["messages"][3]["content"][0]["text"], "two");
    assert_eq!(body["messages"][3]["content"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(body["messages"][1]["content"][0]["signature"], "signed");
    let once = body.clone();
    store.restore(&mut body).unwrap();
    assert_eq!(body, once);
}

#[test]
fn corrupt_linked_history_fails_without_modifying_the_request() {
    for cycle in [false, true] {
        let store = HistoryStore::default();
        let content = record(&store, &[vec![thought("linked")],
            vec![text("one")], vec![text("two")]]);
        let key = content_hash(&content);
        let parent = if cycle { key.clone() } else { "a".repeat(64) };
        let corrupt = json!({"version":2,"parent":parent,
            "turn":{"prompt":1,"parts":[[1,3,6]]}});
        store.0.lock().unwrap().execute("UPDATE checkpoints SET layout=? WHERE fingerprint=?",
            params![corrupt.to_string(),key]).unwrap();
        let mut body = request(content);
        let unchanged = body.clone();
        let error = store.restore(&mut body).unwrap_err();
        assert!(error.contains(if cycle { "cycle" } else { "missing" }), "{error}");
        assert_eq!(body, unchanged);
    }
}

#[test]
fn unrecognized_or_edited_content_is_not_reconstructed_from_a_similar_signature() {
    let store = HistoryStore::default();
    let mut blocks = record(&store, &[vec![thought("signed")], vec![text("original")]]);
    blocks[1] = text("edited by client");
    let mut body = request(blocks);
    let unchanged = body.clone();
    store.restore(&mut body).unwrap();
    assert_eq!(body, unchanged);
}

#[test]
fn storage_limit_keeps_existing_history_and_reports_the_write_failure() {
    let store = HistoryStore::default();
    let mut body = request(record(&store, &[vec![thought("saved")], vec![text("answer")]]));
    {
        let db = store.0.lock().unwrap();
        let pages: i64 = db.pragma_query_value(None, "page_count", |r| r.get(0)).unwrap();
        db.pragma_update(None, "max_page_count", pages).unwrap();
    }
    let error = store.save("next", &json!({"layout":"x".repeat(100000)})).unwrap_err();
    assert!(error.contains("full"), "{error}");
    store.restore(&mut body).unwrap();
    assert_eq!(body["messages"].as_array().unwrap().len(), 5);
}

#[test]
fn preserves_the_recorded_tool_recovery_turn() {
    let store = HistoryStore::default();
    let mut trace = Trace::default();
    let mut content = Vec::new();
    trace.append(&store, &mut content, &[thought("checkpoint")]).unwrap();
    trace.continuing(true);
    let tool = json!({"type":"tool_use","name":"test","id":"test-id","input":{"value":1}});
    trace.append(&store, &mut content, &[tool.clone()]).unwrap();
    let mut body = request(content);
    store.restore(&mut body).unwrap();
    assert_eq!(body["messages"][2]["content"], super::super::messages::checkpoint::TOOL_CONTINUE);
    assert_eq!(body["messages"][3]["content"], json!([tool]));
}

#[test]
fn empty_final_chunk_keeps_the_published_signed_history_restorable() {
    let store = HistoryStore::default();
    let mut trace = Trace::default();
    let mut content = Vec::new();
    trace.append(&store, &mut content, &[thought("empty-final")]).unwrap();
    trace.continuing(false);
    trace.append(&store, &mut content, &[text("complete")]).unwrap();
    let mut expected = request(content.clone());
    store.restore(&mut expected).unwrap();
    trace.append(&store, &mut content, &[]).unwrap();
    let mut body = request(content);
    store.restore(&mut body).unwrap();
    assert_eq!(body, expected);
}

#[cfg(unix)]
#[test]
fn persistent_store_rejects_symlinks_and_public_permissions() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let root = std::env::temp_dir().join(format!("framerize-history-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let link = root.join("linked");
    symlink(&root, &link).unwrap();
    assert!(HistoryStore::open(&link).is_err());
    let private = root.join("private");
    drop(HistoryStore::open(&private).unwrap());
    assert_eq!(std::fs::metadata(private.join("history.sqlite3")).unwrap().permissions().mode() & 0o777, 0o600);
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(HistoryStore::open(&private).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
