//! Inert deletion acknowledgements without changing absent-ID semantics.
use super::*;
use framer_core::account::{AccountState, WorkspaceScope};
use framer_core::ids::{ProjectId, TeamId};
use framer_core::workspace::CreditSnapshot;
use serde_json::{json, Value};

fn install_trigger(store: &SqliteStateStore, sql: &str) {
    store.conn.lock().unwrap().execute_batch(sql).unwrap();
}

#[tokio::test]
async fn ignored_delete_returns_error_without_changing_rows() {
    for malformed in [false, true] {
        let (store, target, _) = fixture(malformed).await;
        let before = snapshot(&store, target.id).await;
        install_trigger(
            &store,
            "CREATE TRIGGER ignore_delete BEFORE DELETE ON accounts
             BEGIN SELECT RAISE(IGNORE); END;",
        );
        let result = store.remove(target.id).await;
        let after = snapshot(&store, target.id).await;
        assert_eq!(result, Err("stored account removal was not applied".into()));
        assert_eq!(after, before, "malformed={malformed}");
    }
}

#[tokio::test]
async fn ordinary_malformed_and_absent_removals_succeed() {
    for malformed in [false, true] {
        let (store, target, peer) = fixture(malformed).await;
        let expected = Snapshot {
            rows: vec![(peer.id.0.to_string(), serde_json::to_string(&peer).unwrap())],
            account: json!({"Ok": null}),
            list: json!({"Ok": [&peer]}),
        };
        for id in [target.id, target.id, AccountId::new()] {
            let result = store.remove(id).await;
            let after = snapshot(&store, id).await;
            assert_eq!(result, Ok(()), "malformed={malformed}");
            assert_eq!(after, expected, "malformed={malformed}");
        }
    }
}

#[tokio::test]
async fn delete_error_keeps_original_message_and_rows() {
    let (store, target, _) = fixture(false).await;
    let before = snapshot(&store, target.id).await;
    install_trigger(
        &store,
        "CREATE TRIGGER reject_delete BEFORE DELETE ON accounts
         BEGIN SELECT RAISE(ABORT, 'fixture rejects delete'); END;",
    );
    let result = store.remove(target.id).await;
    let after = snapshot(&store, target.id).await;
    assert_eq!(result, Err("fixture rejects delete".into()));
    assert_eq!(after, before);
}

async fn fixture(malformed: bool) -> (SqliteStateStore, ManagedAccount, ManagedAccount) {
    let store = SqliteStateStore::open(":memory:").unwrap();
    let target = account("target");
    let peer = account("peer");
    store.upsert(&target).await.unwrap();
    store.upsert(&peer).await.unwrap();
    if malformed {
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE accounts SET json = 'fixture-invalid-json' WHERE id = ?1",
                [target.id.0.to_string()],
            )
            .unwrap();
    }
    (store, target, peer)
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    rows: Vec<(String, String)>,
    account: Value,
    list: Value,
}

fn raw_rows(store: &SqliteStateStore) -> Vec<(String, String)> {
    let conn = store.conn.lock().unwrap();
    let mut statement = conn
        .prepare("SELECT id, json FROM accounts ORDER BY id")
        .unwrap();
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    rows.collect::<Result<Vec<_>, _>>().unwrap()
}

async fn snapshot(store: &SqliteStateStore, id: AccountId) -> Snapshot {
    let rows = raw_rows(store);
    let account = serde_json::to_value(store.get(id).await).unwrap();
    let list = store.list().await.map(|mut accounts| {
        accounts.sort_by_key(|account| account.id.0.to_string());
        accounts
    });
    Snapshot {
        rows,
        account,
        list: serde_json::to_value(list).unwrap(),
    }
}

fn account(label: &str) -> ManagedAccount {
    ManagedAccount {
        id: AccountId::new(),
        alias: label.into(),
        email: Some(format!("{label}@fixture.invalid")),
        framer_user_id: FramerUserId(format!("fixture-user-{label}")),
        castle_device_id: format!("fixture-device-{label}"),
        proxy: Some("http://127.0.0.1:3180".into()),
        current_team: Some(TeamId("primary-team".into())),
        current_project: Some(ProjectId("primary-project".into())),
        extra_workspaces: vec![WorkspaceScope {
            team: TeamId("extra-team".into()),
            project: ProjectId("extra-project".into()),
        }],
        state: AccountState::Ready,
        credit: Some(CreditSnapshot {
            available: 125.5,
            funds: 2,
            grants: 3,
            allocated: 1500.0,
        }),
        consecutive_failures: 3,
        cooldown_step: 2,
        cooling_until_ms: Some(123_456),
    }
}
