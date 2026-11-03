use super::*;
use serde_json::json;

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("framer-daily-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

#[test]
fn a_live_writer_rotates_without_restarting() {
    let root = Temp::new();
    let mut writer = DailyWriter::default();
    writer.event(&root.0, "2026-11-03", &json!({"turn":1})).unwrap();
    writer.event(&root.0, "2026-11-04", &json!({"turn":2})).unwrap();
    writer.event(&root.0, "2026-11-04", &json!({"turn":3})).unwrap();
    let first = std::fs::read_to_string(root.0.join("events-2026-11-03.ndjson")).unwrap();
    let second = std::fs::read_to_string(root.0.join("events-2026-11-04.ndjson")).unwrap();
    assert_eq!(first, "{\"turn\":1}\n");
    assert_eq!(second, "{\"turn\":2}\n{\"turn\":3}\n");
}

#[test]
fn rollover_reapplies_retention_without_removing_other_files() {
    let root = Temp::new();
    let mut writer = DailyWriter::default();
    writer.event(&root.0, "2026-11-03", &json!({})).unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 86400);
    for name in ["events-2020-01-01.ndjson", "keep.txt", "other.ndjson"] {
        let file = File::create(root.0.join(name)).unwrap();
        file.set_modified(old).unwrap();
    }
    writer.event(&root.0, "2026-11-04", &json!({})).unwrap();
    assert!(!root.0.join("events-2020-01-01.ndjson").exists());
    assert!(root.0.join("keep.txt").exists());
    assert!(root.0.join("other.ndjson").exists());
    assert!(root.0.join("events-2026-11-03.ndjson").exists());
}

#[test]
fn retention_keeps_the_current_file_even_when_its_mtime_is_old() {
    let root = Temp::new();
    let path = root.0.join("events-2020-01-01.ndjson");
    let file = File::create(&path).unwrap();
    file.set_modified(std::time::SystemTime::now()
        - std::time::Duration::from_secs(8 * 86400)).unwrap();
    prune(&root.0, "2020-01-01");
    assert!(path.exists());
}

#[test]
fn failed_rollover_is_reported_and_can_recover() {
    let root = Temp::new();
    let mut writer = DailyWriter::default();
    writer.event(&root.0, "2026-11-03", &json!({})).unwrap();
    let blocked = root.0.join("events-2026-11-04.ndjson");
    std::fs::create_dir(&blocked).unwrap();
    assert!(writer.event(&root.0, "2026-11-04", &json!({"lost":true})).is_err());
    assert_eq!(std::fs::read_to_string(root.0.join("events-2026-11-03.ndjson")).unwrap(), "{}\n");
    std::fs::remove_dir(&blocked).unwrap();
    writer.event(&root.0, "2026-11-04", &json!({"recovered":true})).unwrap();
    assert_eq!(std::fs::read_to_string(blocked).unwrap(), "{\"recovered\":true}\n");
}
