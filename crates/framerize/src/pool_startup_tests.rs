use super::*;
use framer_core::StateStore;
use framer_server::{AllAccounts, EligibleAccounts, PoolConfig, ServingAccountPolicy, POOL_SETTING_KEY};

struct Database(std::path::PathBuf);
impl Drop for Database {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

#[test]
fn explicit_startup_flags_override_only_the_requested_saved_fields() {
    let json = r#"{"active_capacity":3,"eligible_accounts":[]}"#;
    let config = resolve_pool_config(PoolConfig::default(), Some(json), Some(6), None).unwrap();
    assert_eq!(config.active_capacity, 6);
    assert_eq!(config.eligible_accounts, EligibleAccounts::Only(vec![]));
    let config = resolve_pool_config(PoolConfig::default(), Some(json), None, Some("all")).unwrap();
    assert_eq!(config.active_capacity, 3);
    assert_eq!(config.eligible_accounts, EligibleAccounts::All(AllAccounts::All));
    let paused = resolve_pool_config(PoolConfig::default(), None, None, Some("none")).unwrap();
    assert_eq!(paused.eligible_accounts, EligibleAccounts::Only(vec![]));
}

#[test]
fn invalid_persisted_or_explicit_configuration_never_falls_back_to_all_accounts() {
    for json in ["invalid", r#"{"active_capacity":0,"eligible_accounts":"all"}"#,
        r#"{"active_capacity":6,"eligible_accounts":"all","unknown":true}"#] {
        assert!(resolve_pool_config(PoolConfig::default(), Some(json), None, None).is_err());
    }
    assert!(resolve_pool_config(PoolConfig::default(), None, Some(65), None).is_err());
    assert!(resolve_pool_config(PoolConfig::default(), None, None, Some("")).is_err());
}

#[tokio::test]
async fn saved_pool_survives_sqlite_reopen_and_supersedes_legacy_allowlist() {
    let path = Database(std::env::temp_dir().join(format!("framerize-pool-{}.db", uuid::Uuid::new_v4())));
    let saved = PoolConfig { active_capacity: 6, eligible_accounts: EligibleAccounts::All(AllAccounts::All) };
    let store = framer_store::SqliteStateStore::open(path.0.to_str().unwrap()).unwrap();
    assert_eq!(store.setting(POOL_SETTING_KEY).await.unwrap(), None);
    store.save_setting(POOL_SETTING_KEY, &serde_json::to_string(&saved).unwrap()).await.unwrap();
    drop(store);
    let store = framer_store::SqliteStateStore::open(path.0.to_str().unwrap()).unwrap();
    let loaded = store.setting(POOL_SETTING_KEY).await.unwrap();
    let legacy = ServingAccountPolicy::parse(Some("00000000-0000-0000-0000-000000000001")).unwrap().config();
    assert_eq!(resolve_pool_config(legacy, loaded.as_deref(), None, None).unwrap(), saved);
}
