use super::*;

#[derive(Default)]
struct Probe {
    writes: Vec<Vec<u8>>,
    flushes: usize,
    limit: Option<usize>,
    write_error: Option<io::ErrorKind>,
    flush_error: Option<io::ErrorKind>,
}

impl Write for Probe {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes.push(bytes.to_vec());
        if let Some(kind) = self.write_error { return Err(kind.into()); }
        Ok(self.limit.unwrap_or(bytes.len()))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        match self.flush_error { Some(kind) => Err(kind.into()), None => Ok(()) }
    }
}

#[test]
fn large_escaped_event_is_one_complete_line() {
    let value = serde_json::json!({"event": "fixture", "text": "é\n\"".repeat(8192)});
    let mut writer = Probe::default();
    event(&mut writer, &value).unwrap();
    assert_eq!(writer.writes.len(), 1);
    let line = &writer.writes[0];
    assert_eq!(line.iter().filter(|&&b| b == b'\n').count(), 1);
    assert_eq!(line.last(), Some(&b'\n'));
    assert_eq!(serde_json::from_slice::<Value>(line).unwrap(), value);
    assert_eq!(writer.flushes, 1);
}

#[test]
fn short_append_never_retries_a_suffix() {
    for limit in [0, 1, 2] {
        let mut writer = Probe { limit: Some(limit), ..Probe::default() };
        assert_eq!(event(&mut writer, &serde_json::json!({})).unwrap_err().kind(),
            io::ErrorKind::WriteZero);
        assert_eq!(writer.writes, vec![b"{}\n".to_vec()]);
        assert_eq!(writer.flushes, 0);
    }
}

#[test]
fn write_errors_do_not_trigger_duplicate_attempts() {
    for kind in [io::ErrorKind::Interrupted, io::ErrorKind::PermissionDenied] {
        let mut writer = Probe { write_error: Some(kind), ..Probe::default() };
        assert_eq!(event(&mut writer, &Value::Null).unwrap_err().kind(), kind);
        assert_eq!(writer.writes, vec![b"null\n".to_vec()]);
        assert_eq!(writer.flushes, 0);
    }
}

#[test]
fn flush_failure_is_returned_without_rewriting_record() {
    let mut writer = Probe {
        flush_error: Some(io::ErrorKind::Other), ..Probe::default()
    };
    assert_eq!(event(&mut writer, &Value::Null).unwrap_err().kind(), io::ErrorKind::Other);
    assert_eq!(writer.writes, vec![b"null\n".to_vec()]);
    assert_eq!(writer.flushes, 1);
}

struct TempFile(std::path::PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

#[test]
fn independent_append_handles_preserve_whole_records() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("event-append-{}-{nonce}", std::process::id()));
    std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
    let file = TempFile(path);
    let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut children = Vec::new();
    for owner in 0..2 {
        let gate = gate.clone();
        let mut output = std::fs::OpenOptions::new().append(true).open(&file.0).unwrap();
        children.push(std::thread::spawn(move || {
            gate.wait();
            for sequence in 0..32 {
                event(&mut output, &serde_json::json!({
                    "owner": owner, "sequence": sequence, "text": "x".repeat(8192)
                })).unwrap();
                std::thread::yield_now();
            }
        }));
    }
    let joined: Vec<_> = children.into_iter().map(|child| child.join()).collect();
    assert!(joined.iter().all(Result::is_ok));
    let contents = std::fs::read_to_string(&file.0).unwrap();
    assert!(contents.ends_with('\n'));
    let mut seen = std::collections::BTreeSet::new();
    for line in contents.lines() {
        let record: Value = serde_json::from_str(line).unwrap();
        assert_eq!(record["text"], "x".repeat(8192));
        let owner = record["owner"].as_u64().unwrap();
        let sequence = record["sequence"].as_u64().unwrap();
        assert!(owner < 2 && sequence < 32 && seen.insert((owner, sequence)));
    }
    assert_eq!(seen.len(), 64);
}
