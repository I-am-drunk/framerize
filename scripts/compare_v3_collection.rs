//! Compare one isolated SDK operation with the production frame decoder.
#[allow(dead_code, unused_imports)]
#[path = "../crates/framer-native/src/castle/mod.rs"]
mod castle;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, io::{Read, Write}, path::Path};

#[allow(dead_code)]
#[path = "../crates/framer-native/src/worker/process/worker_collection.rs"]
mod collection;
#[cfg(test)]
use framer_native::worker;
#[cfg(test)]
#[path = "../crates/framer-native/src/test_support.rs"]
mod test_support;

const SDK: &str = "3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71";
type Checked<T> = Result<T, &'static str>;

fn read(path: &Path, limit: usize) -> Checked<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path).map_err(|_| "input")?.take(limit as u64 + 1)
        .read_to_end(&mut bytes).map_err(|_| "input")?;
    if bytes.len() > limit { return Err("input-size"); }
    Ok(bytes)
}

fn compare(frame: &[u8], snapshot: &Value) -> Checked<Value> {
    if snapshot["schema"] != 4 || snapshot["synthetic"] != true
        || snapshot["sdk_sha256"] != SDK { return Err("snapshot-identity"); }
    let values = collection::decode(frame).ok_or("collection-frame")?;
    let payload: Value = serde_json::from_slice(&frame[12..]).map_err(|_| "collection-json")?;
    if payload["values"] != snapshot["pre_serialization"] { return Err("collection-input"); }
    let expected: Vec<u8> = serde_json::from_value(snapshot["serialized"].clone())
        .map_err(|_| "serialized-input")?;
    let serialized = castle::v3::serialize(&values).map_err(|_| "serialization")?;
    if serialized != expected { return Err("serialization-mismatch"); }
    let timestamp = snapshot["timestamp_ms"].as_i64().ok_or("timestamp")?;
    let mut token = castle::v3::encode_collected(&values, timestamp)
        .map_err(|_| "encoding")?.into_bytes();
    let digest = format!("{:x}", Sha256::digest(&token));
    let token_bytes = token.len();
    token.fill(0);
    if snapshot["token_sha256"].as_str() != Some(digest.as_str())
        || snapshot["token_bytes"].as_u64() != Some(token_bytes as u64) {
        return Err("encoding-mismatch");
    }
    Ok(json!({"schema":1,"synthetic":true,"sdk_sha256":SDK,
        "production_frame_decoded":true,"pre_serialization_equal":true,
        "native_shared_worker_reduced":true,
        "serialized_equal":true,"token_sha256_equal":true,
        "positions":values.len(),"serialized_bytes":serialized.len(),
        "token_bytes":token_bytes,"frame_bytes":frame.len(),
        "frame_sha256":format!("{:x}",Sha256::digest(frame)),
        "token_discarded":true,"live_acceptance":false}))
}

fn negative_checks(frame: &[u8], snapshot: &Value) -> Checked<usize> {
    let mut damaged = frame.to_vec(); damaged[0] ^= 1;
    if compare(&damaged, snapshot).err() != Some("collection-frame")
        || compare(&frame[..frame.len()-1], snapshot).err() != Some("collection-frame") {
        return Err("frame-refusal");
    }
    let mut cases = Vec::new();
    let mut changed = snapshot.clone(); changed["synthetic"] = json!(false);
    cases.push((changed, "snapshot-identity"));
    let mut changed = snapshot.clone(); changed["pre_serialization"] = Value::Null;
    cases.push((changed, "collection-input"));
    let mut changed = snapshot.clone();
    changed["serialized"][0] = json!(snapshot["serialized"][0].as_u64().unwrap() ^ 1);
    cases.push((changed, "serialization-mismatch"));
    let mut changed = snapshot.clone(); changed["token_sha256"] = json!("0".repeat(64));
    cases.push((changed, "encoding-mismatch"));
    let mut changed = snapshot.clone();
    changed["timestamp_ms"] = json!(snapshot["timestamp_ms"].as_i64().unwrap() + 1);
    cases.push((changed, "encoding-mismatch"));
    for (changed, expected) in &cases {
        if compare(frame, changed).err() != Some(*expected) { return Err("mismatch-refusal"); }
    }
    let mut payload: Value = serde_json::from_slice(&frame[12..]).unwrap();
    let different = snapshot["pre_serialization"]["values"][242].to_string() + "-changed";
    payload["shared_worker_reply"] = json!({"kind":"string",
        "units":different.encode_utf16().collect::<Vec<_>>()});
    for expected in ["serialization-mismatch", "collection-frame"] {
        let body = serde_json::to_vec(&payload).unwrap();
        let mut changed = frame[..8].to_vec();
        changed.extend_from_slice(&(body.len() as u32).to_be_bytes());
        changed.extend(body);
        if compare(&changed, snapshot).err() != Some(expected) { return Err("reply-refusal"); }
        payload.as_object_mut().unwrap().remove("shared_worker_reply");
    }
    Ok(cases.len() + 4)
}

fn run() -> Checked<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 { return Err("arguments: FRAME SNAPSHOT REPORT"); }
    let mut frame = read(Path::new(&args[0]), collection::LIMIT + 12)?;
    let raw = read(Path::new(&args[1]), 8 * 1024 * 1024)?;
    let snapshot: Value = serde_json::from_slice(&raw).map_err(|_| "snapshot-json")?;
    let mut report = compare(&frame, &snapshot)?;
    report["negative_checks"] = json!(negative_checks(&frame, &snapshot)?);
    report["snapshot_sha256"] = json!(format!("{:x}", Sha256::digest(&raw)));
    frame.fill(0);
    let mut file = fs::OpenOptions::new().write(true).create_new(true)
        .open(&args[2]).map_err(|_| "report")?;
    file.write_all(serde_json::to_string_pretty(&report).unwrap().as_bytes())
        .map_err(|_| "report")?;
    println!("PASS: fresh collection frame, native reply reduction and encoding match; 9 negative checks pass");
    Ok(())
}

fn main() {
    if let Err(phase) = run() {
        eprintln!("FAIL: {phase}");
        std::process::exit(1);
    }
}
