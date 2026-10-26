use super::*;
use framer_core::account::AccountState;

fn account() -> ManagedAccount {
    ManagedAccount {
        id: AccountId::new(),
        alias: "sqlite fixture".into(),
        email: None,
        framer_user_id: FramerUserId("fixture-user".into()),
        castle_device_id: "fixture-device".into(),
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

#[tokio::test]
async fn missing_account_is_none() {
    let store = SqliteStateStore::open(":memory:").unwrap();
    assert!(matches!(store.get(AccountId::new()).await, Ok(None)));
}

#[tokio::test]
async fn query_failure_is_not_a_missing_account() {
    let store = SqliteStateStore::open(":memory:").unwrap();
    store.conn.lock().unwrap().execute_batch("DROP TABLE accounts").unwrap();
    assert!(store.get(AccountId::new()).await.is_err());
}

#[tokio::test]
async fn non_text_row_is_not_a_missing_account() {
    let store = SqliteStateStore::open(":memory:").unwrap();
    let id = AccountId::new();
    store.conn.lock().unwrap().execute(
        "INSERT INTO accounts (id, json) VALUES (?1, X'00')", [id.0.to_string()],
    ).unwrap();
    assert!(store.get(id).await.is_err());
}

#[tokio::test]
async fn invalid_account_json_remains_an_error() {
    let store = SqliteStateStore::open(":memory:").unwrap();
    let id = AccountId::new();
    store.conn.lock().unwrap().execute(
        "INSERT INTO accounts (id, json) VALUES (?1, 'invalid-json')", [id.0.to_string()],
    ).unwrap();
    assert!(store.get(id).await.is_err());
}

#[tokio::test]
async fn saved_account_roundtrips_without_changes() {
    let store = SqliteStateStore::open(":memory:").unwrap();
    let account = account();
    store.upsert(&account).await.unwrap();
    let actual = store.get(account.id).await.unwrap().unwrap();
    assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(account).unwrap());
}

#[tokio::test]
async fn rejected_update_preserves_previous_account() {
    let store = SqliteStateStore::open(":memory:").unwrap();
    let original = account();
    store.upsert(&original).await.unwrap();
    store.conn.lock().unwrap().execute_batch(
        "CREATE TRIGGER reject_update AFTER UPDATE ON accounts
         BEGIN SELECT RAISE(ABORT, 'fixture rejects update'); END;",
    ).unwrap();
    let mut changed = original.clone();
    changed.alias = "rejected alias".into();
    changed.consecutive_failures = 4;
    changed.cooling_until_ms = Some(123456);
    assert!(store.upsert(&changed).await.is_err());
    let actual = store.get(original.id).await.unwrap().unwrap();
    assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(original).unwrap());
}
