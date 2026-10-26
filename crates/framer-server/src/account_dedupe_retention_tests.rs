//! Dedupe retains a recoverable device owner after discarding a dead peer.
use super::*;

struct DedupeStore {
    ports: Arc<FixturePorts>,
    mutations: Arc<Mutex<std::collections::HashSet<AccountId>>>,
    removed: Mutex<Vec<AccountId>>,
}

#[async_trait::async_trait]
impl StateStore for DedupeStore {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        self.ports.get(id).await
    }

    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        self.ports.list().await
    }

    async fn upsert(&self, _account: &ManagedAccount) -> Result<(), String> {
        self.ports.unexpected("state")
    }

    async fn remove(&self, id: AccountId) -> Result<(), String> {
        assert!(self.mutations.lock().unwrap().contains(&id));
        self.removed.lock().unwrap().push(id);
        self.ports.state_writes.fetch_add(1, Ordering::SeqCst);
        self.ports.records.lock().unwrap().retain(|a| a.id != id);
        Ok(())
    }
}

fn stored_rows(store: &DedupeStore) -> Value {
    serde_json::to_value(&*store.ports.records.lock().unwrap()).unwrap()
}

async fn sweep(rows: Vec<ManagedAccount>, apply: bool) -> (Value, Arc<DedupeStore>) {
    let (mut state, ports) = fixture(false);
    *ports.records.lock().unwrap() = rows;
    *ports.native_session.lock().unwrap() = Some(None);
    let store = Arc::new(DedupeStore {
        ports: ports.clone(),
        mutations: state.serving_accounts.mutations.clone(),
        removed: Mutex::new(Vec::new()),
    });
    state.state = store.clone();
    let config = state.serving_accounts.config();
    let outcome =
        crate::dedupe_accounts_observed(State(state.clone()), Json(crate::DedupeBody { apply }))
            .await;
    assert!(outcome.cleanup_confirmed);
    let report = outcome.result.unwrap().0;
    assert_eq!(report["applied"], apply);
    assert_eq!(state.serving_accounts.config(), config);
    assert!(state.serving_accounts.mutations.lock().unwrap().is_empty());
    assert!(state
        .serving_accounts
        .residents
        .snapshot()
        .unwrap()
        .residents
        .is_empty());
    assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.client_calls.load(Ordering::SeqCst), 0);
    assert!(ports.saved.lock().unwrap().is_empty());
    assert!(ports.registered_proxies.lock().unwrap().is_empty());
    let removed = store.removed.lock().unwrap().len();
    assert_eq!(ports.state_reads.load(Ordering::SeqCst), 1 + removed);
    assert_eq!(ports.state_writes.load(Ordering::SeqCst), removed);
    (report, store)
}

fn dead_and_cooling() -> (ManagedAccount, ManagedAccount) {
    let mut dead = account(1);
    dead.current_project = None;
    dead.castle_device_id = "shared-device-retention".into();
    let mut cooling = account(2);
    cooling.castle_device_id = dead.castle_device_id.clone();
    cooling.cooling_until_ms = Some(u64::MAX);
    cooling.consecutive_failures = 1;
    (dead, cooling)
}

fn assert_cooling_report(report: &Value) {
    assert_eq!(
        report["keeping"],
        json!([{
            "name": "fixture-2@example.invalid", "team": "team-2", "device": "shared-d"
        }])
    );
    assert_eq!(
        report["removing"],
        json!([{
            "name": "fixture-1@example.invalid",
            "reason": "no project, no credits, no session — nothing to recover"
        }])
    );
    assert_eq!(report["needs_project"], json!([]));
}

#[tokio::test(flavor = "current_thread")]
async fn preview_keeps_cooling_project_after_dead_device_peer() {
    let (dead, cooling) = dead_and_cooling();
    // Input order is reversed; the unchanged sort puts the dead peer first.
    let rows = vec![cooling, dead];
    let before = serde_json::to_value(&rows).unwrap();
    let (report, store) = sweep(rows, false).await;
    assert_cooling_report(&report);
    assert_eq!(stored_rows(&store), before);
    assert!(store.removed.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn apply_removes_dead_peer_and_preserves_cooling_project_record() {
    let (dead, cooling) = dead_and_cooling();
    let expected = serde_json::to_value(vec![cooling.clone()]).unwrap();
    let dead_id = dead.id;
    let (report, store) = sweep(vec![cooling, dead], true).await;
    assert_cooling_report(&report);
    assert_eq!(stored_rows(&store), expected);
    assert_eq!(*store.removed.lock().unwrap(), vec![dead_id]);
}

#[tokio::test(flavor = "current_thread")]
async fn retained_device_owner_still_eliminates_real_clone() {
    let kept = account(1);
    let mut clone = account(2);
    clone.castle_device_id = kept.castle_device_id.clone();
    clone.consecutive_failures = 1;
    for apply in [false, true] {
        let rows = vec![clone.clone(), kept.clone()];
        let expected = serde_json::to_value(if apply {
            vec![kept.clone()]
        } else {
            rows.clone()
        })
        .unwrap();
        let (report, store) = sweep(rows, apply).await;
        assert_eq!(
            report["keeping"],
            json!([{
                "name": "fixture-1@example.invalid", "team": "team-1", "device": "device-1"
            }])
        );
        assert_eq!(
            report["removing"],
            json!([{
                "name": "fixture-2@example.invalid",
                "reason": "shares a Castle device id with another record"
            }])
        );
        assert_eq!(report["needs_project"], json!([]));
        assert_eq!(stored_rows(&store), expected);
        assert_eq!(
            *store.removed.lock().unwrap(),
            if apply { vec![clone.id] } else { Vec::new() }
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn all_dead_device_peers_remain_removable() {
    let (dead, mut other) = dead_and_cooling();
    other.current_project = None;
    for apply in [false, true] {
        let rows = vec![other.clone(), dead.clone()];
        let expected = serde_json::to_value(if apply { Vec::new() } else { rows.clone() }).unwrap();
        let (report, store) = sweep(rows, apply).await;
        assert_eq!(report["keeping"], json!([]));
        assert_eq!(report["needs_project"], json!([]));
        let names: Vec<_> = report["removing"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["fixture-1@example.invalid", "fixture-2@example.invalid"]
        );
        assert_eq!(stored_rows(&store), expected);
        assert_eq!(
            *store.removed.lock().unwrap(),
            if apply {
                vec![dead.id, other.id]
            } else {
                Vec::new()
            }
        );
    }
}
