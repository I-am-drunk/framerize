use super::*;

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("framerize-gate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}
fn config() -> GateConfig {
    GateConfig { initial: 5, min: 1, max: 8, recover_ms: 60_000,
        relax_ms: 60_000, min_interval_ms: 0 }
}

#[tokio::test]
async fn persisted_ceiling_limits_real_permits_after_restart() {
    let dir = Directory::new();
    let path = dir.0.join("limits.json");
    std::fs::write(&path, r#"{"workspace:model":2}"#).unwrap();
    let gate = ConcurrencyGate::with_state_file(config(), path.clone()).unwrap();
    assert_eq!(gate.limit_of("workspace:model").await, 2);
    let first = gate.acquire_now("workspace:model").await.unwrap();
    let second = gate.acquire("workspace:model").await;
    assert!(gate.acquire_now("workspace:model").await.is_none());
    drop((first, second));
    gate.on_overloaded("workspace:model").await;
    let restarted = ConcurrencyGate::with_state_file(config(), path).unwrap();
    assert_eq!(restarted.limit_of("workspace:model").await, 1);
    assert_eq!(ConcurrencyGate::with_config(config()).limit_of("workspace:model").await, 5);
}

#[tokio::test]
async fn explicit_files_are_independent_and_keep_other_saved_workspaces() {
    let dir = Directory::new();
    let path = dir.0.join("first.json");
    std::fs::write(&path, r#"{"saved:model":3}"#).unwrap();
    let first = ConcurrencyGate::with_state_file(config(), path.clone()).unwrap();
    let second_path = dir.0.join("second.json");
    let second = ConcurrencyGate::with_state_file(config(), second_path.clone()).unwrap();
    first.on_overloaded("new:model").await;
    let saved = state::load(&path).unwrap();
    assert_eq!(saved["saved:model"], 3);
    assert_eq!(saved["new:model"], 4);
    assert_eq!(second.limit_of("new:model").await, 5);
    assert!(!second_path.exists());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn invalid_state_is_reported_without_replacing_it() {
    let dir = Directory::new();
    let path = dir.0.join("limits.json");
    std::fs::write(&path, "broken").unwrap();
    assert!(ConcurrencyGate::with_state_file(config(), path.clone()).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
}
