//! Stored keys must identify the account returned to an ownership-checked caller.
use super::*;
use framer_core::account::{AccountState, WorkspaceScope};
use framer_core::ids::{ProjectId, TeamId};
use framer_core::workspace::CreditSnapshot;
use serde_json::json;
use std::path::PathBuf;

const IDENTITY_ERROR: &str = "stored account identity does not match its key";
type RawRow = (i64, String, Option<Vec<u8>>, String, Option<Vec<u8>>);

fn account(n: usize) -> ManagedAccount {
    ManagedAccount {
        id: AccountId(format!("{n:08x}-bcde-4000-8000-000000000001").parse().unwrap()),
        alias: format!("fixture-{n}"),
        email: Some(format!("fixture-{n}@example.invalid")),
        framer_user_id: FramerUserId(format!("fixture-user-{n}")),
        castle_device_id: format!("fixture-device-{n}"),
        proxy: Some(format!("http://proxy-{n}.invalid:8080")),
        current_team: Some(TeamId(format!("primary-team-{n}"))),
        current_project: Some(ProjectId(format!("primary-project-{n}"))),
        extra_workspaces: vec![WorkspaceScope {
            team: TeamId(format!("extra-team-{n}")),
            project: ProjectId(format!("extra-project-{n}")),
        }],
        state: AccountState::Ready,
        credit: Some(CreditSnapshot {
            available: 87.5, funds: 2, grants: 3, allocated: 1500.0,
        }),
        consecutive_failures: 2,
        cooldown_step: 3,
        cooling_until_ms: Some(99_000), cooldown_cause: None,
    }
}

fn insert(connection: &Connection, key: &str, raw: &str) {
    connection.execute("INSERT INTO accounts (id, json) VALUES (?1, ?2)",
        (key, raw)).unwrap();
}

fn rows(connection: &Connection) -> Vec<RawRow> {
    let mut query = connection.prepare(
        "SELECT rowid, typeof(id), CAST(id AS BLOB), typeof(json), CAST(json AS BLOB)
         FROM accounts ORDER BY rowid",
    ).unwrap();
    let records = query.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?,
        r.get(3)?, r.get(4)?))).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    records
}

fn snapshot(store: &SqliteStateStore) -> Vec<RawRow> {
    rows(&store.conn.lock().unwrap())
}

#[tokio::test]
async fn get_rejects_different_embedded_account_without_rewrite() {
    let store = SqliteStateStore::open(":memory:").unwrap();
    let key = account(1).id;
    let valid = account(2);
    let mut misplaced = valid.clone();
    misplaced.alias = "different record under the wrong key".into();
    store.upsert(&valid).await.unwrap();
    insert(&store.conn.lock().unwrap(), &key.0.to_string(),
        &serde_json::to_string(&misplaced).unwrap());
    let before = snapshot(&store);
    let error = store.get(key).await.expect_err("get must not return another account");
    assert_eq!(error, IDENTITY_ERROR);
    assert_eq!(json!(store.get(valid.id).await.unwrap().unwrap()), json!(valid));
    assert_eq!(snapshot(&store), before);
}

#[tokio::test]
async fn list_rejects_mismatched_account_without_partial_result() {
    for invalid_first in [true, false] {
        let store = SqliteStateStore::open(":memory:").unwrap();
        let valid = account(3);
        let misplaced = account(2);
        let good = (valid.id.0.to_string(), serde_json::to_string(&valid).unwrap());
        let bad = (account(1).id.0.to_string(), serde_json::to_string(&misplaced).unwrap());
        let records = if invalid_first { [bad, good] } else { [good, bad] };
        for (key, raw) in records { insert(&store.conn.lock().unwrap(), &key, &raw); }
        let before = snapshot(&store);
        let error = store.list().await.expect_err("list must refuse an inconsistent row key");
        assert_eq!(error, IDENTITY_ERROR);
        assert_eq!(json!(store.get(valid.id).await.unwrap().unwrap()), json!(valid));
        assert_eq!(snapshot(&store), before);
    }
}

#[tokio::test]
async fn list_rejects_noncanonical_or_invalid_keys() {
    for kind in ["uppercase", "compact", "invalid-text", "null", "blob"] {
        let store = SqliteStateStore::open(":memory:").unwrap();
        let record = account(4);
        let raw = serde_json::to_string(&record).unwrap();
        {
            let connection = store.conn.lock().unwrap();
            let key = record.id.0.to_string();
            match kind {
                "uppercase" => insert(&connection, &key.to_uppercase(), &raw),
                "compact" => insert(&connection, &key.replace('-', ""), &raw),
                "invalid-text" => {
                    connection.execute(
                        "INSERT INTO accounts (id, json) VALUES (CAST(X'80' AS TEXT), ?1)",
                        [&raw],
                    ).unwrap();
                }
                "null" => {
                    connection.execute("INSERT INTO accounts (id, json) VALUES (NULL, ?1)",
                        [&raw]).unwrap();
                }
                "blob" => {
                    connection.execute("INSERT INTO accounts (id, json) VALUES (X'80', ?1)",
                        [&raw]).unwrap();
                }
                _ => unreachable!(),
            }
        }
        let before = snapshot(&store);
        let error = store.list().await.expect_err("list must refuse a key it cannot look up");
        assert_eq!(error, IDENTITY_ERROR, "{kind}");
        assert!(store.get(record.id).await.unwrap().is_none(), "{kind}");
        assert_eq!(snapshot(&store), before, "{kind}");
    }
}

struct Database(PathBuf);

impl Database {
    fn new() -> Self {
        let directory = std::env::temp_dir()
            .join(format!("framerize-key-identity-{}", AccountId::new().0));
        std::fs::create_dir(&directory).unwrap();
        let database = Self(directory.join("state.sqlite"));
        database.connection().execute_batch(
            "CREATE TABLE accounts (id TEXT PRIMARY KEY, json TEXT NOT NULL)",
        ).unwrap();
        database
    }

    fn connection(&self) -> Connection { Connection::open(&self.0).unwrap() }

    fn open(&self) -> SqliteStateStore {
        SqliteStateStore::open(self.0.to_str().unwrap()).unwrap()
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

#[tokio::test]
async fn legacy_migration_does_not_hide_key_mismatch() {
    let database = Database::new();
    let key = account(5).id;
    let original = account(6);
    let mut legacy = json!(original);
    legacy.as_object_mut().unwrap().remove("castle_device_id");
    legacy["future_metadata"] = json!({"keep": ["unchanged", 42, true]});
    insert(&database.connection(), &key.0.to_string(),
        &serde_json::to_string_pretty(&legacy).unwrap());
    let store = database.open();
    let migrated = snapshot(&store);
    let mut fields: serde_json::Value = serde_json::from_slice(
        migrated[0].4.as_ref().unwrap(),
    ).unwrap();
    let device = fields.as_object_mut().unwrap().remove("castle_device_id").unwrap();
    assert!(!device.as_str().unwrap().is_empty());
    assert_eq!(fields, legacy, "migration must preserve identity and unknown fields");
    let error = store.get(key).await.expect_err("migrating a device must not bless a wrong key");
    assert_eq!(error, IDENTITY_ERROR);
    assert_eq!(store.list().await.unwrap_err(), IDENTITY_ERROR);
    assert!(store.get(original.id).await.unwrap().is_none());
    assert_eq!(snapshot(&store), migrated);
    drop(store);
    let reopened = database.open();
    assert_eq!(reopened.get(key).await.unwrap_err(), IDENTITY_ERROR);
    assert_eq!(reopened.list().await.unwrap_err(), IDENTITY_ERROR);
    assert_eq!(snapshot(&reopened), migrated, "reopen must preserve the generated device and key");
}

#[tokio::test]
async fn canonical_key_accepts_valid_embedded_uuid_spellings() {
    for uppercase in [false, true] {
        let store = SqliteStateStore::open(":memory:").unwrap();
        let expected = account(7);
        let mut value = json!(expected);
        if uppercase { value["id"] = json!(expected.id.0.to_string().to_uppercase()); }
        value["future_metadata"] = json!({"keep": ["unknown", 91, false]});
        insert(&store.conn.lock().unwrap(), &expected.id.0.to_string(),
            &serde_json::to_string_pretty(&value).unwrap());
        let before = snapshot(&store);
        assert_eq!(json!(store.get(expected.id).await.unwrap().unwrap()), json!(expected));
        assert_eq!(json!(store.list().await.unwrap()), json!([expected]));
        assert_eq!(snapshot(&store), before, "valid JSON formatting and fields must survive");
    }
}
