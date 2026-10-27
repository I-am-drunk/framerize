//! Inert SQLite acknowledgements for ignored inserts and conflict updates.
use super::*;
use framer_core::account::{AccountState, WorkspaceScope};
use framer_core::ids::{ProjectId, TeamId};
use framer_core::workspace::CreditSnapshot;

const WRITE_ERROR: &str = "stored account write was not applied";

#[tokio::test]
async fn ordinary_insert_identical_update_and_changed_update_succeed() {
    let store = SqliteStateStore::open(":memory:").unwrap();
    let peer = account("peer");
    store.upsert(&peer).await.unwrap();
    let original = account("original");
    for (stage, current) in [
        ("insert", original.clone()),
        ("identical update", original.clone()),
        ("changed update", replacement(&original)),
    ] {
        let result = store.upsert(&current).await;
        let stored = store.get(current.id).await.unwrap();
        let after_raw = raw_rows(&store);
        let after_decoded = decoded_rows(&store).await;
        let mut expected_raw = vec![
            (peer.id.0.to_string(), serde_json::to_string(&peer).unwrap()),
            (
                current.id.0.to_string(),
                serde_json::to_string(&current).unwrap(),
            ),
        ];
        expected_raw.sort();
        let expected_decoded: Vec<serde_json::Value> = expected_raw
            .iter()
            .map(|(_, json)| serde_json::from_str(json).unwrap())
            .collect();
        assert_eq!(result, Ok(()), "{stage}");
        assert_eq!(after_raw, expected_raw, "{stage}");
        assert_eq!(after_decoded, expected_decoded, "{stage}");
        assert_eq!(
            serde_json::to_value(stored.unwrap()).unwrap(),
            serde_json::to_value(&current).unwrap(),
            "{stage}",
        );
    }
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

async fn decoded_rows(store: &SqliteStateStore) -> Vec<serde_json::Value> {
    let mut rows = store.list().await.unwrap();
    rows.sort_by_key(|account| account.id.0.to_string());
    rows.iter()
        .map(|row| serde_json::to_value(row).unwrap())
        .collect()
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
            team: TeamId(format!("extra-team-{label}")),
            project: ProjectId(format!("extra-project-{label}")),
        }],
        state: AccountState::CoolingDown,
        credit: Some(CreditSnapshot {
            available: 47.5,
            funds: 2,
            grants: 3,
            allocated: 1500.0,
        }),
        consecutive_failures: 4,
        cooldown_step: 2,
        cooling_until_ms: Some(123_456),
    }
}

fn replacement(original: &ManagedAccount) -> ManagedAccount {
    let mut changed = account("replacement");
    changed.id = original.id;
    changed.proxy = None;
    changed.current_team = Some(TeamId("replacement-primary-team".into()));
    changed.current_project = Some(ProjectId("replacement-primary-project".into()));
    changed.state = AccountState::Ready;
    changed.credit = None;
    changed.consecutive_failures = 0;
    changed.cooldown_step = 0;
    changed.cooling_until_ms = None;
    changed
}

#[tokio::test]
async fn ignored_insert_returns_error_without_changing_rows() {
    let store = SqliteStateStore::open(":memory:").unwrap();
    let peer = account("peer");
    store.upsert(&peer).await.unwrap();
    let before_raw = raw_rows(&store);
    let before_decoded = decoded_rows(&store).await;
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER ignore_insert BEFORE INSERT ON accounts
         BEGIN SELECT RAISE(IGNORE); END;",
        )
        .unwrap();
    let incoming = account("incoming");
    let result = store.upsert(&incoming).await;
    let stored = store.get(incoming.id).await.unwrap();
    let stored_peer = store.get(peer.id).await.unwrap();
    let after_raw = raw_rows(&store);
    let after_decoded = decoded_rows(&store).await;

    assert_eq!(result, Err(WRITE_ERROR.into()));
    assert!(stored.is_none());
    assert_eq!(after_raw, before_raw);
    assert_eq!(after_decoded, before_decoded);
    assert_eq!(
        serde_json::to_value(stored_peer.unwrap()).unwrap(),
        serde_json::to_value(&peer).unwrap(),
    );
}

#[tokio::test]
async fn ignored_conflict_update_returns_error_and_preserves_complete_account() {
    let store = SqliteStateStore::open(":memory:").unwrap();
    let original = account("original");
    let peer = account("peer");
    store.upsert(&original).await.unwrap();
    store.upsert(&peer).await.unwrap();
    let before_raw = raw_rows(&store);
    let before_decoded = decoded_rows(&store).await;
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER ignore_update BEFORE UPDATE ON accounts
         BEGIN SELECT RAISE(IGNORE); END;",
        )
        .unwrap();
    let result = store.upsert(&replacement(&original)).await;
    let stored = store.get(original.id).await.unwrap();
    let stored_peer = store.get(peer.id).await.unwrap();
    let after_raw = raw_rows(&store);
    let after_decoded = decoded_rows(&store).await;

    assert_eq!(result, Err(WRITE_ERROR.into()));
    assert_eq!(after_raw, before_raw);
    assert_eq!(after_decoded, before_decoded);
    assert_eq!(
        serde_json::to_value(stored.unwrap()).unwrap(),
        serde_json::to_value(&original).unwrap(),
    );
    assert_eq!(
        serde_json::to_value(stored_peer.unwrap()).unwrap(),
        serde_json::to_value(&peer).unwrap(),
    );
}
