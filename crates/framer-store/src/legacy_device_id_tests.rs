use super::*;
use serde_json::json;
use std::path::PathBuf;

struct Database(PathBuf);

impl Database {
    fn new() -> Self {
        let directory = std::env::temp_dir()
            .join(format!("framerize-device-migration-{}", AccountId::new().0));
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

    fn insert(&self, id: AccountId, record: &str) {
        self.connection().execute("INSERT INTO accounts (id, json) VALUES (?1, ?2)",
            (id.0.to_string(), record)).unwrap();
    }

    fn raw(&self, id: AccountId) -> String {
        self.connection().query_row("SELECT json FROM accounts WHERE id = ?1",
            [id.0.to_string()], |row| row.get(0)).unwrap()
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

fn legacy(id: AccountId) -> String {
    json!({
        "id": id.0, "alias": "legacy fixture", "framer_user_id": "fixture-user",
        "current_team": "fixture-team", "current_project": "fixture-project",
        "state": "ready", "credit": null, "consecutive_failures": 0,
        "cooldown_step": 0,
    }).to_string()
}

#[tokio::test]
async fn legacy_identity_is_saved_before_reads_and_survives_reopen() {
    let database = Database::new();
    let id = AccountId::new();
    database.insert(id, &legacy(id));
    let store = database.open().unwrap();
    let persisted = database.raw(id);
    let record: serde_json::Value = serde_json::from_str(&persisted).unwrap();
    let device = record["castle_device_id"].as_str().unwrap();
    assert!(!device.is_empty());
    for _ in 0..2 {
        assert_eq!(store.get(id).await.unwrap().unwrap().castle_device_id, device);
        let accounts = store.list().await.unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].castle_device_id, device);
    }
    drop(store);
    let reopened = database.open().unwrap();
    assert_eq!(reopened.get(id).await.unwrap().unwrap().castle_device_id, device);
    assert_eq!(database.raw(id), persisted);
}

#[tokio::test]
async fn legacy_accounts_keep_distinct_identities() {
    let database = Database::new();
    let first = AccountId::new();
    let second = AccountId::new();
    database.insert(first, &legacy(first));
    database.insert(second, &legacy(second));
    let store = database.open().unwrap();
    let a = store.get(first).await.unwrap().unwrap();
    let b = store.get(second).await.unwrap().unwrap();
    assert_ne!(a.castle_device_id, b.castle_device_id);
    let accounts = store.list().await.unwrap();
    assert_eq!(accounts.iter().find(|a| a.id == first).unwrap().castle_device_id,
        a.castle_device_id);
    assert_eq!(accounts.iter().find(|a| a.id == second).unwrap().castle_device_id,
        b.castle_device_id);
}

#[tokio::test]
async fn migrated_identity_allows_scope_checked_updates() {
    let database = Database::new();
    let id = AccountId::new();
    database.insert(id, &legacy(id));
    let store = database.open().unwrap();
    let before = store.get(id).await.unwrap().unwrap();
    let updated = framer_core::pipeline::update_account(&store,
        &tokio::sync::Mutex::new(()), &before, |account| account.alias = "updated".into())
        .await.unwrap();
    assert_eq!(updated.castle_device_id, before.castle_device_id);
    assert_eq!(store.get(id).await.unwrap().unwrap().alias, "updated");
}

#[test]
fn existing_device_fields_are_not_rewritten() {
    let database = Database::new();
    let mut records = Vec::new();
    for device in [json!("saved-device"), json!(""), json!(null)] {
        let id = AccountId::new();
        let mut record: serde_json::Value = serde_json::from_str(&legacy(id)).unwrap();
        record["castle_device_id"] = device;
        let raw = serde_json::to_string_pretty(&record).unwrap() + "\n";
        database.insert(id, &raw);
        records.push((id, raw));
    }
    database.connection().execute_batch(
        "CREATE TRIGGER reject_rewrite AFTER UPDATE ON accounts
         BEGIN SELECT RAISE(ABORT, 'existing device must not be rewritten'); END;",
    ).unwrap();
    drop(database.open().unwrap());
    for (id, raw) in records {
        assert_eq!(database.raw(id), raw);
    }
}

#[tokio::test]
async fn unknown_fields_keep_their_exact_json_bytes() {
    let database = Database::new();
    let id = AccountId::new();
    let mut raw = legacy(id);
    raw.pop();
    raw.push_str(r#", "future": {"counter":1234567890123456789012345678901234567890,"large":1e400,"escaped":"\u0041","minus_zero":-0} } "#);
    raw.push('\n');
    let closing = raw.trim_end().len() - 1;
    database.insert(id, &raw);
    let store = database.open().unwrap();
    let after = database.raw(id);
    assert!(after.starts_with(&raw[..closing]));
    assert!(after.ends_with(&raw[closing..]));
    let account = store.get(id).await.unwrap().unwrap();
    let inserted = &after[closing..after.len() - raw[closing..].len()];
    assert_eq!(inserted, format!(",\"castle_device_id\":{}",
        serde_json::to_string(&account.castle_device_id).unwrap()));
}

#[test]
fn failing_later_row_rolls_back_earlier_migrations() {
    let database = Database::new();
    let first = AccountId::new();
    let second = AccountId::new();
    let original_first = legacy(first);
    let original_second = legacy(second);
    database.insert(first, &original_first);
    database.insert(second, &original_second);
    database.connection().execute_batch(&format!(
        "CREATE TRIGGER reject_second AFTER UPDATE ON accounts
         WHEN OLD.id = '{}'
         BEGIN SELECT RAISE(ABORT, 'fixture rejects second migration'); END;",
        second.0,
    )).unwrap();
    let error = database.open().err().expect("migration must fail");
    assert!(error.contains("fixture rejects second migration"));
    assert_eq!(database.raw(first), original_first);
    assert_eq!(database.raw(second), original_second);
    database.connection().execute_batch("DROP TRIGGER reject_second").unwrap();
    drop(database.open().unwrap());
    assert_ne!(database.raw(first), original_first);
    assert_ne!(database.raw(second), original_second);
}

#[test]
fn ignored_write_prevents_opening_with_an_unpersisted_identity() {
    let database = Database::new();
    let id = AccountId::new();
    let original = legacy(id);
    database.insert(id, &original);
    database.connection().execute_batch(
        "CREATE TRIGGER ignore_migration BEFORE UPDATE ON accounts
         BEGIN SELECT RAISE(IGNORE); END;",
    ).unwrap();
    assert!(database.open().is_err());
    assert_eq!(database.raw(id), original);
}

#[test]
fn invalid_records_are_left_unchanged() {
    let database = Database::new();
    let malformed = AccountId::new();
    let incomplete = AccountId::new();
    let non_text = AccountId::new();
    database.insert(malformed, "not-json");
    database.insert(incomplete, r#"{"alias":"missing required fields"}"#);
    database.connection().execute("INSERT INTO accounts (id, json) VALUES (?1, X'00')",
        [non_text.0.to_string()]).unwrap();
    drop(database.open().unwrap());
    assert_eq!(database.raw(malformed), "not-json");
    assert_eq!(database.raw(incomplete), r#"{"alias":"missing required fields"}"#);
    let bytes: Vec<u8> = database.connection().query_row(
        "SELECT json FROM accounts WHERE id = ?1", [non_text.0.to_string()],
        |row| row.get(0)).unwrap();
    assert_eq!(bytes, vec![0]);
}

#[test]
fn failed_commit_rolls_back_generated_identities() {
    let database = Database::new();
    let id = AccountId::new();
    let original = legacy(id);
    database.insert(id, &original);
    let mut connection = database.connection();
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE parents (id INTEGER PRIMARY KEY);
         CREATE TABLE children (parent INTEGER REFERENCES parents(id)
             DEFERRABLE INITIALLY DEFERRED);
         CREATE TRIGGER fail_commit AFTER UPDATE ON accounts
         BEGIN INSERT INTO children (parent) VALUES (1); END;",
    ).unwrap();
    let error = legacy_device_ids::persist(&mut connection).unwrap_err();
    assert!(error.contains("FOREIGN KEY constraint failed"));
    assert!(connection.is_autocommit());
    let children: u32 = connection.query_row("SELECT count(*) FROM children", [],
        |row| row.get(0)).unwrap();
    assert_eq!(children, 0);
    assert_eq!(database.raw(id), original);
}
