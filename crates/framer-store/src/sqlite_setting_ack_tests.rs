//! Inert settings persistence acknowledgements for ignored SQLite writes.
use super::*;

const TARGET: &str = " fixture-policy ";
const PEER: &str = "unrelated-setting";
const OLD: &str = " {\"active\":2} \n";
const NEW: &str = "{\"active\":4}";
const PEER_VALUE: &str = " unchanged snowman: \u{2603} \n";
const WRITE_ERROR: &str = "stored setting write was not applied";

#[tokio::test]
async fn ignored_setting_insert_returns_error_without_persisting() {
    let store = fixture(false).await;
    let before = snapshot(&store).await;
    install_trigger(
        &store,
        "CREATE TRIGGER ignore_setting_insert BEFORE INSERT ON settings
         BEGIN SELECT RAISE(IGNORE); END;",
    );
    let result = store.save_setting(TARGET, NEW).await;
    let after = snapshot(&store).await;
    assert_eq!(result, Err(WRITE_ERROR.into()));
    assert_eq!(after, before);
}

#[tokio::test]
async fn ignored_setting_update_returns_error_and_preserves_values() {
    let store = fixture(true).await;
    let before = snapshot(&store).await;
    install_trigger(
        &store,
        "CREATE TRIGGER ignore_setting_update BEFORE UPDATE ON settings
         BEGIN SELECT RAISE(IGNORE); END;",
    );
    let result = store.save_setting(TARGET, NEW).await;
    let after = snapshot(&store).await;
    assert_eq!(result, Err(WRITE_ERROR.into()));
    assert_eq!(after, before);
}

#[tokio::test]
async fn ordinary_identical_changed_and_empty_settings_preserve_bytes() {
    let store = fixture(false).await;
    for value in [OLD, OLD, NEW, ""] {
        let result = store.save_setting(TARGET, value).await;
        let after = snapshot(&store).await;
        let trimmed = store.setting(TARGET.trim()).await.unwrap();
        let mut rows = vec![
            (TARGET.into(), value.into()),
            (PEER.into(), PEER_VALUE.into()),
        ];
        rows.sort();
        let expected = Snapshot {
            rows,
            target: Some(value.into()),
            peer: Some(PEER_VALUE.into()),
        };
        assert_eq!(result, Ok(()));
        assert_eq!(after, expected);
        assert!(trimmed.is_none());
    }
}

#[tokio::test]
async fn setting_sql_errors_keep_their_original_message_and_rows() {
    for (existing, trigger) in [
        (
            false,
            "CREATE TRIGGER reject_setting BEFORE INSERT ON settings
                 BEGIN SELECT RAISE(ABORT, 'fixture rejects setting'); END;",
        ),
        (
            true,
            "CREATE TRIGGER reject_setting BEFORE UPDATE ON settings
                BEGIN SELECT RAISE(ABORT, 'fixture rejects setting'); END;",
        ),
    ] {
        let store = fixture(existing).await;
        let before = snapshot(&store).await;
        install_trigger(&store, trigger);
        let result = store.save_setting(TARGET, NEW).await;
        let after = snapshot(&store).await;
        assert_eq!(result, Err("fixture rejects setting".into()));
        assert_eq!(after, before);
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    rows: Vec<(String, String)>,
    target: Option<String>,
    peer: Option<String>,
}

fn raw_rows(store: &SqliteStateStore) -> Vec<(String, String)> {
    let conn = store.conn.lock().unwrap();
    let mut statement = conn
        .prepare("SELECT key, value FROM settings ORDER BY key")
        .unwrap();
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    rows.collect::<Result<Vec<_>, _>>().unwrap()
}

fn install_trigger(store: &SqliteStateStore, sql: &str) {
    store.conn.lock().unwrap().execute_batch(sql).unwrap();
}

async fn fixture(existing: bool) -> SqliteStateStore {
    let store = SqliteStateStore::open(":memory:").unwrap();
    store.save_setting(PEER, PEER_VALUE).await.unwrap();
    if existing {
        store.save_setting(TARGET, OLD).await.unwrap();
    }
    store
}

async fn snapshot(store: &SqliteStateStore) -> Snapshot {
    Snapshot {
        rows: raw_rows(store),
        target: store.setting(TARGET).await.unwrap(),
        peer: store.setting(PEER).await.unwrap(),
    }
}
