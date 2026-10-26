//! An unreadable peer must not disappear from an otherwise successful list.
use super::*;
use framer_core::account::{AccountState, WorkspaceScope};
use framer_core::ids::{ProjectId, TeamId};
use framer_core::workspace::CreditSnapshot;
use rusqlite::types::Value as SqlValue;
use serde_json::json;

const DECODE_ERROR: &str = "stored account could not be decoded";
const PRIVATE_MARKER: &str = "fixture-value-must-stay-private";

fn account(n: usize) -> ManagedAccount {
    ManagedAccount {
        id: AccountId::new(),
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
        cooling_until_ms: Some(99_000),
    }
}

fn insert(store: &SqliteStateStore, account: &ManagedAccount, raw: &str) {
    store.conn.lock().unwrap().execute(
        "INSERT INTO accounts (id, json) VALUES (?1, ?2)",
        rusqlite::params![account.id.0.to_string(), raw],
    ).unwrap();
}

fn rows(store: &SqliteStateStore) -> Vec<(String, SqlValue)> {
    let connection = store.conn.lock().unwrap();
    let mut query = connection.prepare("SELECT id, json FROM accounts ORDER BY id").unwrap();
    let records = query.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    records
}

async fn reject_mixed_rows(schema_error: bool) {
    for invalid_first in [true, false] {
        let store = SqliteStateStore::open(":memory:").unwrap();
        let valid = account(1);
        let invalid = account(2);
        let good_json = serde_json::to_string(&valid).unwrap();
        let bad_json = if schema_error {
            let mut value = serde_json::to_value(&invalid).unwrap();
            value["state"] = json!(PRIVATE_MARKER);
            serde_json::to_string(&value).unwrap()
        } else { ["{", PRIVATE_MARKER].concat() };
        let records = if invalid_first {
            [(&invalid, &bad_json), (&valid, &good_json)]
        } else { [(&valid, &good_json), (&invalid, &bad_json)] };
        for (account, raw) in records { insert(&store, account, raw); }
        let before = rows(&store);
        let error = store.list().await.expect_err("an invalid peer must refuse the whole list");
        assert_eq!(error, DECODE_ERROR);
        assert!(!error.contains(PRIVATE_MARKER));
        assert_eq!(rows(&store), before, "list must not repair or discard stored rows");
        assert_eq!(json!(store.get(valid.id).await.unwrap().unwrap()), json!(valid));
    }
}

#[tokio::test]
async fn mixed_syntax_error_refuses_partial_accounts() { reject_mixed_rows(false).await; }

#[tokio::test]
async fn mixed_schema_error_refuses_partial_accounts() { reject_mixed_rows(true).await; }

#[tokio::test]
async fn valid_accounts_are_all_returned_without_rewrite() {
    let store = SqliteStateStore::open(":memory:").unwrap();
    let first = account(3);
    let mut second = account(4);
    second.email = None;
    second.proxy = None;
    second.current_team = None;
    second.current_project = None;
    second.extra_workspaces.clear();
    second.state = AccountState::Disconnected;
    second.credit = None;
    second.consecutive_failures = 0;
    second.cooldown_step = 0;
    second.cooling_until_ms = None;
    let mut value = serde_json::to_value(&first).unwrap();
    value["future_metadata"] = json!({"keep": ["unknown", 42, true]});
    insert(&store, &first, &serde_json::to_string_pretty(&value).unwrap());
    insert(&store, &second, &serde_json::to_string(&second).unwrap());
    let before = rows(&store);
    let mut actual = store.list().await.unwrap();
    let mut expected = vec![first, second];
    actual.sort_by_key(|account| account.id.0);
    expected.sort_by_key(|account| account.id.0);
    assert_eq!(json!(actual), json!(expected));
    assert_eq!(rows(&store), before, "unknown fields and formatting must survive");
}

#[tokio::test]
async fn empty_store_returns_empty_accounts() {
    let store = SqliteStateStore::open(":memory:").unwrap();
    assert!(store.list().await.unwrap().is_empty());
    assert!(rows(&store).is_empty());
}

#[tokio::test]
async fn sqlite_query_failure_remains_error() {
    let store = SqliteStateStore::open(":memory:").unwrap();
    let expected = {
        let connection = store.conn.lock().unwrap();
        connection.execute("DROP TABLE accounts", []).unwrap();
        let error = connection.prepare("SELECT json FROM accounts")
            .err().expect("missing table").to_string();
        error
    };
    let error = store.list().await.unwrap_err();
    assert_eq!(error, expected);
    assert_ne!(error, DECODE_ERROR);
    assert!(store.conn.lock().unwrap().prepare("SELECT json FROM accounts").is_err());
}

#[tokio::test]
async fn non_text_account_remains_error() {
    let store = SqliteStateStore::open(":memory:").unwrap();
    let valid = account(5);
    let invalid = account(6);
    insert(&store, &valid, &serde_json::to_string(&valid).unwrap());
    let expected = {
        let connection = store.conn.lock().unwrap();
        connection.execute("INSERT INTO accounts (id, json) VALUES (?1, X'00')",
            [invalid.id.0.to_string()]).unwrap();
        connection.query_row("SELECT json FROM accounts WHERE id = ?1",
            [invalid.id.0.to_string()], |row| row.get::<_, String>(0))
            .unwrap_err().to_string()
    };
    let before = rows(&store);
    let error = store.list().await.unwrap_err();
    assert_eq!(error, expected);
    assert_ne!(error, DECODE_ERROR);
    assert_eq!(rows(&store), before);
}
