//! In-memory poison handling only. No files, keyring, tasks or live state.
use super::*;
use framer_core::account::AccountState;
use std::panic::{catch_unwind, AssertUnwindSafe};

struct Fixture {
    store: SqliteStateStore,
    account: ManagedAccount,
    raw: String,
}

fn account() -> ManagedAccount {
    ManagedAccount {
        id: AccountId::new(),
        alias: "poison fixture".into(),
        email: None,
        framer_user_id: FramerUserId("poison-user".into()),
        castle_device_id: "poison-device".into(),
        proxy: None,
        current_team: None,
        current_project: None,
        extra_workspaces: Vec::new(),
        state: AccountState::Disconnected,
        credit: None,
        consecutive_failures: 0,
        cooldown_step: 0,
        cooling_until_ms: None,
    }
}

impl Fixture {
    fn new() -> Self {
        let store = SqliteStateStore::open(":memory:").unwrap();
        let account = account();
        let mut value = serde_json::to_value(&account).unwrap();
        value["unknown_fixture_field"] = serde_json::json!({"keep": true});
        let raw = serde_json::to_string(&value).unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, json) VALUES (?1, ?2)",
                (account.id.0.to_string(), &raw),
            )
            .unwrap();
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('fixture', 'before')",
                [],
            )
            .unwrap();
        }
        Self {
            store,
            account,
            raw,
        }
    }

    fn poison(&self) -> String {
        let panic = catch_unwind(AssertUnwindSafe(|| {
            let _guard = self.store.conn.lock().unwrap();
            panic!("deliberate fixture poison");
        }));
        assert!(panic.is_err());
        assert!(self.store.conn.is_poisoned());
        self.store.conn.lock().err().unwrap().to_string()
    }

    fn assert_unchanged(&self) {
        assert!(self.store.conn.is_poisoned());
        // Test-only inspection; production never takes a poisoned guard.
        let conn = self.store.conn.lock().err().unwrap().into_inner();
        assert_eq!(
            rows(&conn, "SELECT id, json FROM accounts ORDER BY id"),
            vec![(self.account.id.0.to_string(), self.raw.clone())]
        );
        assert_eq!(
            rows(&conn, "SELECT key, value FROM settings ORDER BY key"),
            vec![("fixture".into(), "before".into())]
        );
    }
}

fn rows(conn: &Connection, sql: &str) -> Vec<(String, String)> {
    let mut stmt = conn.prepare(sql).unwrap();
    let mapped = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    let values: rusqlite::Result<Vec<_>> = mapped.collect();
    values.unwrap()
}

fn assert_lock_error<T>(result: Result<T, String>, expected: &str) {
    match result {
        Ok(_) => panic!("poisoned store operation unexpectedly succeeded"),
        Err(actual) => assert_eq!(actual, expected),
    }
}

#[tokio::test]
async fn poisoned_get_returns_original_lock_error() {
    let f = Fixture::new();
    let error = f.poison();
    assert_lock_error(f.store.get(f.account.id).await, &error);
    f.assert_unchanged();
}

#[tokio::test]
async fn poisoned_list_returns_original_lock_error() {
    let f = Fixture::new();
    let error = f.poison();
    assert_lock_error(f.store.list().await, &error);
    f.assert_unchanged();
}

#[tokio::test]
async fn poisoned_upsert_returns_lock_error_without_writing() {
    let f = Fixture::new();
    let mut changed = f.account.clone();
    changed.alias = "must not persist".into();
    let error = f.poison();
    assert_lock_error(f.store.upsert(&changed).await, &error);
    f.assert_unchanged();
}

#[tokio::test]
async fn poisoned_remove_returns_lock_error_without_deleting() {
    let f = Fixture::new();
    let error = f.poison();
    assert_lock_error(f.store.remove(f.account.id).await, &error);
    f.assert_unchanged();
}

#[tokio::test]
async fn poisoned_settings_keep_original_lock_error_behavior() {
    let f = Fixture::new();
    let error = f.poison();
    assert_lock_error(f.store.setting("fixture").await, &error);
    assert_lock_error(
        f.store.save_setting("fixture", "must not persist").await,
        &error,
    );
    f.assert_unchanged();
}
