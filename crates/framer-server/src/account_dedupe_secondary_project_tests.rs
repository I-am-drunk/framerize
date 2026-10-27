//! Saved secondary projects keep a record recoverable without a primary project.
use super::*;
use framer_core::account::WorkspaceScope;

#[tokio::test(flavor = "current_thread")]
async fn preview_retains_secondary_projects_without_primary() {
    for primary_team in [false, true] {
        let row = secondary_record(primary_team);
        let result = sweep(vec![row.clone()], false).await;
        assert_retained(&result, &row);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn apply_retains_secondary_projects_without_primary() {
    for primary_team in [false, true] {
        let row = secondary_record(primary_team);
        let result = sweep(vec![row.clone()], true).await;
        assert_retained(&result, &row);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn secondary_project_keeper_preserves_clone_rule() {
    for apply in [false, true] {
        let keeper = secondary_record(false);
        let mut clone = account(2);
        clone.current_project = None;
        clone.current_team = None;
        clone.castle_device_id = keeper.castle_device_id.clone();
        clone.consecutive_failures = 1;
        let rows = vec![clone.clone(), keeper.clone()];
        let result = sweep(rows.clone(), apply).await;
        assert_eq!(
            result.rows,
            if apply { json!([keeper]) } else { json!(rows) }
        );
        assert_eq!(
            result.report["keeping"],
            json!([{
                "name": "fixture-1@example.invalid", "team": null, "device": "device-1",
            }])
        );
        assert_eq!(
            result.report["removing"],
            json!([{
                "name": "fixture-2@example.invalid",
                "reason": "shares a Castle device id with another record",
            }])
        );
        assert_eq!(
            result.report["needs_project"],
            json!([{
                "name": "fixture-1@example.invalid", "credits": null, "has_session": false,
                "reason": "no project yet — provision it, do not delete it",
            }])
        );
        assert_eq!(result.removed, if apply { vec![clone.id] } else { vec![] });
        assert_eq!(result.state_reads, if apply { 3 } else { 1 });
        assert_eq!(result.session_reads, 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn empty_secondary_scopes_preserve_dead_rule() {
    for apply in [false, true] {
        let mut row = secondary_record(false);
        row.extra_workspaces.clear();
        let result = sweep(vec![row.clone()], apply).await;
        assert_eq!(result.rows, if apply { json!([]) } else { json!([row]) });
        assert_eq!(result.report["keeping"], json!([]));
        assert_eq!(result.report["needs_project"], json!([]));
        assert_eq!(
            result.report["removing"],
            json!([{
                "name": "fixture-1@example.invalid",
                "reason": "no project, no credits, no session — nothing to recover",
            }])
        );
        assert_eq!(result.removed, if apply { vec![row.id] } else { vec![] });
        assert_eq!(result.state_reads, if apply { 2 } else { 1 });
        assert_eq!(result.session_reads, 1);
    }
}

struct Sweep {
    report: Value,
    rows: Value,
    removed: Vec<AccountId>,
    state_reads: usize,
    session_reads: usize,
}

async fn sweep(rows: Vec<ManagedAccount>, apply: bool) -> Sweep {
    let (mut state, ports) = fixture(false);
    *ports.records.lock().unwrap() = rows;
    *ports.native_session.lock().unwrap() = Some(None);
    let store = Arc::new(Store {
        ports: ports.clone(),
        mutations: state.serving_accounts.mutations.clone(),
        removed: Mutex::new(Vec::new()),
    });
    state.runtime_mut().expect("unique fixture runtime").state = store.clone();
    let config = state.serving_accounts.config();
    // No task is spawned: this bound covers the fully awaited handler.
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        crate::dedupe_accounts_observed(State(state.clone()), Json(crate::DedupeBody { apply })),
    )
    .await
    .expect("bounded dedupe fixture");
    assert!(outcome.cleanup_confirmed);
    let report = outcome.result.expect("dedupe result").0;
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
    let removed = store.removed.lock().unwrap().clone();
    assert_eq!(ports.state_writes.load(Ordering::SeqCst), removed.len());
    let rows = serde_json::to_value(&*ports.records.lock().unwrap()).unwrap();
    Sweep {
        report,
        rows,
        removed,
        state_reads: ports.state_reads.load(Ordering::SeqCst),
        session_reads: ports.session_calls.load(Ordering::SeqCst),
    }
}

fn assert_retained(result: &Sweep, row: &ManagedAccount) {
    assert_eq!(result.rows, json!([row]));
    assert_eq!(result.report["removing"], json!([]));
    assert_eq!(
        result.report["keeping"],
        json!([{
            "name": "fixture-1@example.invalid",
            "team": row.current_team.as_ref().map(|team| &team.0),
            "device": "device-1",
        }])
    );
    assert_eq!(
        result.report["needs_project"],
        json!([{
            "name": "fixture-1@example.invalid",
            "credits": row.credit.map(|credit| credit.available),
            "has_session": false,
            "reason": "no project yet — provision it, do not delete it",
        }])
    );
    assert!(result.removed.is_empty());
    assert_eq!(result.state_reads, 1);
    assert_eq!(result.session_reads, 1);
    assert!(!row.is_eligible());
    assert_eq!(row.all_scopes().len(), 2);
}

fn secondary_record(primary_team: bool) -> ManagedAccount {
    let mut row = account(1);
    row.current_project = None;
    if !primary_team {
        row.current_team = None;
    }
    row.credit = primary_team.then_some(CreditSnapshot {
        available: 0.0,
        funds: 0,
        grants: 0,
        allocated: 0.0,
    });
    row.extra_workspaces = vec![
        WorkspaceScope {
            team: TeamId("saved-secondary-team-a".into()),
            project: ProjectId("saved-secondary-project-a".into()),
        },
        WorkspaceScope {
            team: TeamId("saved-secondary-team-b".into()),
            project: ProjectId("saved-secondary-project-b".into()),
        },
    ];
    row
}

struct Store {
    ports: Arc<FixturePorts>,
    mutations: Arc<Mutex<std::collections::HashSet<AccountId>>>,
    removed: Mutex<Vec<AccountId>>,
}

#[async_trait::async_trait]
impl StateStore for Store {
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
        self.ports
            .records
            .lock()
            .unwrap()
            .retain(|row| row.id != id);
        self.ports.state_writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
