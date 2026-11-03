use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Mutex};

pub(crate) struct HistoryStore(Mutex<Connection>);

const MAX_LAYOUT_BYTES: usize = 262144;
const MAX_TURNS: usize = 1024;
pub(crate) const WRITE_VERSION: u8 = if cfg!(feature = "legacy-checkpoint-writes") { 1 } else { 2 };

#[path = "messages_history_layout.rs"]
mod layout;

#[cfg(test)]
#[path = "messages_history_tests.rs"]
mod tests;

fn failure(error: impl std::fmt::Display) -> String {
    format!("Claude checkpoint history could not be preserved: {error}")
}

fn hash(value: &Value) -> String {
    // serde_json's default ordered map gives stable object-key ordering.
    format!("{:x}", Sha256::digest(serde_json::to_vec(value).expect("JSON value")))
}

fn anchor(blocks: &[Value]) -> Option<String> {
    blocks.iter().find(|b| match b["type"].as_str() {
        Some("thinking") => b["signature"].as_str().is_some_and(|s| !s.is_empty()),
        Some("redacted_thinking") => b["data"].as_str().is_some_and(|s| !s.is_empty()),
        _ => false,
    }).map(|b| hash(&without_cache_marker(b)))
}

fn without_cache_marker(block: &Value) -> Value {
    let mut copy = block.clone();
    if let Some(object) = copy.as_object_mut() { object.remove("cache_control"); }
    copy
}

fn content_hash(blocks: &[Value]) -> String {
    hash(&Value::Array(blocks.iter().map(without_cache_marker).collect()))
}

#[derive(Default)]
pub(super) struct Trace {
    turns: Vec<Value>,
    prompt: u8,
    previous: Option<String>,
    layout_bytes: usize,
}

impl Trace {
    pub(super) fn append(&mut self, store: &HistoryStore, combined: &mut Vec<Value>,
        incoming: &[Value]) -> Result<(), String> {
        // Empty text can merge into the previous block without changing it.
        // Keep the published layout instead of linking a hash to itself.
        if incoming.is_empty() || (combined.last().is_some_and(plain_text)
            && incoming.iter().all(|block| plain_text(block) && block["text"] == "")) {
            return Ok(());
        }
        let mut parts = Vec::new();
        for block in incoming {
            let plain = plain_text(block);
            let joins = plain && combined.last().is_some_and(plain_text);
            let index = if joins { combined.len() - 1 } else { combined.len() };
            let start = if joins { combined[index]["text"].as_str().unwrap().len() } else { 0 };
            parts.push(if plain {
                json!([index, start, start + block["text"].as_str().unwrap().len()])
            } else { json!([index]) });
            crate::protocol::messages::merge_blocks(combined, vec![block.clone()]);
        }
        let turn = json!({"parts":parts,"prompt":self.prompt});
        self.layout_bytes += serde_json::to_vec(&turn).expect("JSON value").len() + 1;
        if self.turns.len() >= MAX_TURNS || self.layout_bytes + 32 > MAX_LAYOUT_BYTES {
            return Err(failure("checkpoint layout exceeds its size limit"));
        }
        self.turns.push(turn.clone());
        if (self.turns.len() > 1 || incoming != combined.as_slice()) && anchor(combined).is_some() {
            // Only offsets, hashes and fixed continuation kinds are persisted.
            // Keep each published prefix, sharing its already-persisted layout.
            let saved = match &self.previous {
                Some(parent) if WRITE_VERSION == 2 => json!({"version":2,"parent":parent,"turn":turn}),
                _ => json!({"version":1,"turns":self.turns}),
            };
            let key = content_hash(combined);
            store.save(&key, &saved)?;
            self.previous = Some(key);
        }
        Ok(())
    }

    pub(super) fn continuing(&mut self, tool: bool) {
        self.prompt = if tool { 2 } else { 1 };
    }
}

fn plain_text(block: &Value) -> bool {
    block["type"] == "text" && block["text"].is_string()
        && block.as_object().is_some_and(|o| o.keys().all(|k| k == "type" || k == "text"))
}

fn expand(message: &Value, blocks: &[Value], layout: &Value) -> Result<Vec<Value>, String> {
    let invalid = || failure("stored checkpoint layout is invalid");
    if layout["version"] != 1 { return Err(invalid()); }
    let turns = layout["turns"].as_array().ok_or_else(invalid)?;
    let count = turns.len();
    if count == 0 || count > MAX_TURNS { return Err(invalid()); }
    let mut output = Vec::new();
    let mut merged = Vec::new();
    for (index, turn) in turns[..count].iter().enumerate() {
        let prompt = turn["prompt"].as_u64().ok_or_else(invalid)?;
        let text = match (index, prompt) {
            (0, 0) => None,
            (1.., 1) => Some(super::messages::checkpoint::CONTINUE),
            (1.., 2) => Some(super::messages::checkpoint::TOOL_CONTINUE),
            _ => return Err(invalid()),
        };
        if let Some(text) = text { output.push(json!({"role":"user","content":text})); }
        let parts = turn["parts"].as_array().ok_or_else(invalid)?;
        let content: Vec<Value> = parts.iter().map(|p| piece(blocks, p)).collect::<Result<_, _>>()?;
        crate::protocol::messages::merge_blocks(&mut merged, content.clone());
        let mut assistant = message.clone();
        assistant["content"] = json!(content);
        output.push(assistant);
    }
    let normalize = |values: &[Value]| {
        let mut normalized = Vec::new();
        crate::protocol::messages::merge_blocks(&mut normalized,
            values.iter().map(without_cache_marker).collect());
        normalized
    };
    if normalize(&merged) != normalize(blocks) { return Err(invalid()); }
    Ok(output)
}

fn piece(blocks: &[Value], part: &Value) -> Result<Value, String> {
    let invalid = || failure("stored checkpoint block offsets are invalid");
    let fields = part.as_array().ok_or_else(invalid)?;
    let index = |n: usize| fields.get(n).and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok()).ok_or_else(invalid);
    let block = blocks.get(index(0)?).ok_or_else(invalid)?;
    match fields.len() {
        1 => Ok(block.clone()),
        3 if plain_text(&without_cache_marker(block)) => {
            let text = block["text"].as_str().unwrap();
            let end = index(2)?;
            let slice = text.get(index(1)?..end).ok_or_else(invalid)?;
            let mut value = json!({"type":"text","text":slice});
            if end == text.len() {
                if let Some(marker) = block.get("cache_control") { value["cache_control"] = marker.clone(); }
            }
            Ok(value)
        },
        _ => Err(invalid()),
    }
}

impl Default for HistoryStore {
    fn default() -> Self {
        Self::initialize(Connection::open_in_memory().expect("history database"))
            .expect("history schema")
    }
}

impl HistoryStore {
    pub(super) fn restore(&self, payload: &mut Value) -> Result<(), String> {
        let Some(messages) = payload["messages"].as_array() else { return Ok(()); };
        let mut restored = Vec::new();
        for message in messages {
            let blocks = message["content"].as_array();
            let key = blocks.and_then(|b| anchor(b).map(|_| content_hash(b)));
            let layout = if message["role"] == "assistant" {
                key.as_deref().map(|k| self.load(k)).transpose()?.flatten()
            } else { None };
            match (blocks, layout) {
                (Some(blocks), Some(layout)) => restored.extend(expand(message, blocks, &layout)?),
                _ => restored.push(message.clone()),
            }
        }
        payload["messages"] = Value::Array(restored);
        Ok(())
    }

    fn initialize(db: Connection) -> Result<Self, String> {
        db.execute_batch("PRAGMA page_size=4096; PRAGMA cache_size=-1024; PRAGMA synchronous=FULL;
            PRAGMA max_page_count=32768; PRAGMA journal_size_limit=0;
            CREATE TABLE IF NOT EXISTS checkpoints (
                fingerprint TEXT PRIMARY KEY, layout TEXT NOT NULL);")
            .map_err(failure)?;
        let page_size: u64 = db.pragma_query_value(None, "page_size", |r| r.get(0)).map_err(failure)?;
        let max_pages: u64 = db.pragma_query_value(None, "max_page_count", |r| r.get(0)).map_err(failure)?;
        if page_size != 4096 || max_pages > 32768 {
            return Err(failure("database exceeds the 128 MiB checkpoint-storage limit"));
        }
        Ok(Self(Mutex::new(db)))
    }

    pub(crate) fn open(directory: &Path) -> Result<Self, String> {
        super::messages_history_file::prepare(directory)?;
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW;
        Self::initialize(Connection::open_with_flags(directory.join("history.sqlite3"), flags)
            .map_err(failure)?)
    }

    fn load(&self, key: &str) -> Result<Option<Value>, String> {
        let db = self.0.lock().map_err(|_| failure("history lock was poisoned"))?;
        layout::read(&db, key)?.map(|value| layout::resolve(&db, value)).transpose()
    }

    fn save(&self, key: &str, layout: &Value) -> Result<(), String> {
        let raw = serde_json::to_string(layout).map_err(failure)?;
        if raw.len() > MAX_LAYOUT_BYTES { return Err(failure("layout exceeds 256 KiB")); }
        if layout["version"] == 2 && layout["parent"] == key {
            return Err(failure("checkpoint made no content progress"));
        }
        let db = self.0.lock().map_err(|_| failure("history lock was poisoned"))?;
        db.execute("INSERT INTO checkpoints VALUES (?,?) ON CONFLICT DO NOTHING",
            params![key, raw]).map_err(failure)?;
        let identical: bool = db.query_row("SELECT layout=? FROM checkpoints WHERE fingerprint=?",
            params![raw, key], |r| r.get(0)).map_err(failure)?;
        if !identical {
            // An older release may have saved the same turns without links.
            // Keep that row and compare the actual layout, not its encoding.
            let existing = layout::read(&db, key)?.ok_or_else(|| failure("missing saved layout"))?;
            if layout::resolve(&db, existing)? != layout::resolve(&db, layout.clone())? {
                return Err(failure("conflicting layout for the same signed content"));
            }
        }
        Ok(())
    }
}
