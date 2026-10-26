//! Migration skips undecodable text without hiding later read errors.
use crate::{AccountId, Connection, ManagedAccount, SqliteStateStore, StateStore};
use serde_json::json;
use std::path::PathBuf;

type RawRow = (i64, String, Vec<u8>, String, Vec<u8>);
struct Database(PathBuf);

impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

fn legacy(id: AccountId, alias: &str) -> String {
    json!({
        "id": id.0, "alias": alias, "email": "fixture@example.invalid",
        "framer_user_id": "fixture-user", "proxy": "http://fixture.invalid:8080",
        "current_team": "fixture-team", "current_project": "fixture-project",
        "extra_workspaces": [{"team": "extra-team", "project": "extra-project"}],
        "state": "ready", "credit": {
            "available": 87.5, "funds": 2, "grants": 3, "allocated": 1500.0,
        },
        "consecutive_failures": 2, "cooldown_step": 3, "cooling_until_ms": 99_000,
    }).to_string()
}

impl Database {
    fn new() -> Self {
        let directory = std::env::temp_dir()
            .join(format!("framerize-device-encoding-{}", AccountId::new().0));
        std::fs::create_dir(&directory).unwrap();
        let database = Self(directory.join("state.sqlite"));
        database.connection().execute_batch(
            "CREATE TABLE accounts (id TEXT PRIMARY KEY, json TEXT NOT NULL)",
        ).unwrap();
        database
    }

    fn connection(&self) -> Connection {
        Connection::open(&self.0).unwrap()
    }

    fn open(&self) -> Result<SqliteStateStore, String> {
        SqliteStateStore::open(self.0.to_str().unwrap())
    }

    fn raw(&self, id: AccountId) -> String {
        self.connection().query_row("SELECT json FROM accounts WHERE id = ?1",
            [id.0.to_string()], |row| row.get(0)).unwrap()
    }

    fn read_error(&self, id: AccountId) -> String {
        self.connection().query_row("SELECT json FROM accounts WHERE id = ?1",
            [id.0.to_string()], |row| row.get::<_, String>(0)).unwrap_err().to_string()
    }

    fn rows(&self) -> Vec<RawRow> {
        let connection = self.connection();
        let mut query = connection.prepare(
            "SELECT rowid, typeof(id), CAST(id AS BLOB), typeof(json), CAST(json AS BLOB)
             FROM accounts ORDER BY rowid",
        ).unwrap();
        let rows = query.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?,
            row.get(3)?, row.get(4)?))).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        rows
    }

    fn insert(&self, id: AccountId, raw: &str) {
        self.connection().execute("INSERT INTO accounts (id, json) VALUES (?1, ?2)",
            (id.0.to_string(), raw)).unwrap();
    }

    fn migrated(&self, id: AccountId, original: &str) -> ManagedAccount {
        let saved = self.raw(id);
        let account: ManagedAccount = serde_json::from_str(&saved).unwrap();
        assert!(!account.castle_device_id.is_empty());
        let closing = original.trim_end().len() - 1;
        assert_eq!(saved, format!("{},\"castle_device_id\":{}{}",
            &original[..closing], serde_json::to_string(&account.castle_device_id).unwrap(),
            &original[closing..]));
        account
    }

    fn insert_invalid(&self, id: AccountId) {
        self.connection().execute(
            "INSERT INTO accounts (id, json) VALUES (?1, CAST(X'80FF' AS TEXT))",
            [id.0.to_string()],
        ).unwrap();
    }
}

#[tokio::test]
async fn invalid_utf8_rows_preserve_valid_migration_and_read_errors() {
    for invalid_first in [true, false] {
        let database = Database::new();
        let valid = AccountId::new();
        let invalid = AccountId::new();
        let original = legacy(valid, "valid alongside invalid text");
        if invalid_first { database.insert_invalid(invalid); }
        database.insert(valid, &original);
        if !invalid_first { database.insert_invalid(invalid); }
        let before = database.rows();
        let invalid_index = if invalid_first { 0 } else { 1 };
        assert_eq!(before.len(), 2);
        assert_eq!(before[invalid_index].3, "text");
        assert_eq!(before[invalid_index].4, [0x80, 0xff]);
        let expected_error = database.read_error(invalid);
        let store = database.open().unwrap();
        let account = database.migrated(valid, &original);
        let after = database.rows();
        assert_eq!(after[invalid_index], before[invalid_index]);
        assert_eq!(json!(store.get(valid).await.unwrap().unwrap()), json!(account));
        assert_eq!(store.get(invalid).await.unwrap_err(), expected_error);
        assert_eq!(store.list().await.unwrap_err(), expected_error);
        assert_eq!(database.rows(), after);
        drop(store);

        let reopened = database.open().unwrap();
        assert_eq!(json!(reopened.get(valid).await.unwrap().unwrap()), json!(account));
        assert_eq!(reopened.get(invalid).await.unwrap_err(), expected_error);
        assert_eq!(reopened.list().await.unwrap_err(), expected_error);
        assert_eq!(database.rows(), after);
    }
}

#[test]
fn invalid_utf8_skip_preserves_late_migration_rollback() {
    let database = Database::new();
    let first = AccountId::new();
    let invalid = AccountId::new();
    let later = AccountId::new();
    database.insert(first, &legacy(first, "migrated before rejection"));
    database.insert_invalid(invalid);
    database.insert(later, &legacy(later, "rejected later migration"));
    let before = database.rows();
    database.connection().execute_batch(&format!(
        "CREATE TRIGGER reject_later BEFORE UPDATE ON accounts
         WHEN OLD.id = '{}'
         BEGIN SELECT RAISE(ABORT, 'fixture rejects later encoding migration'); END;",
        later.0,
    )).unwrap();
    let expected_error = database.connection().execute(
        "UPDATE accounts SET json = json WHERE id = ?1", [later.0.to_string()],
    ).unwrap_err().to_string();
    assert_eq!(database.rows(), before);
    let error = database.open().err().expect("later SQL error must prevent opening");
    assert_eq!(error, expected_error);
    assert_eq!(database.rows(), before);
}

#[tokio::test]
async fn valid_multibyte_legacy_json_preserves_unknown_fields() {
    let database = Database::new();
    let id = AccountId::new();
    let mut original = legacy(id, "café 東京 🧭");
    original.pop();
    original.push_str(r#", "future": {"label":"naïve 雪","escaped":"\u96ea","counter":123456789012345678901234567890} } "#);
    original.push('\n');
    database.insert(id, &original);
    let store = database.open().unwrap();
    let account = database.migrated(id, &original);
    assert_eq!(account.alias, "café 東京 🧭");
    let after = database.rows();
    assert_eq!(json!(store.get(id).await.unwrap().unwrap()), json!(account));
    assert_eq!(json!(store.list().await.unwrap()), json!([&account]));
    assert_eq!(database.rows(), after);
    drop(store);

    let reopened = database.open().unwrap();
    assert_eq!(json!(reopened.get(id).await.unwrap().unwrap()), json!(account));
    assert_eq!(json!(reopened.list().await.unwrap()), json!([&account]));
    assert_eq!(database.rows(), after);
}
