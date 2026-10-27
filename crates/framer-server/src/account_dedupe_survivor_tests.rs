//! Inert dedupe races. Every concurrent future is bounded and joined before assertions.
use super::*;
use framer_core::browser_lifecycle::{
    BrowserAccountState, BrowserAccountStatus, IdleCloseOutcome, IdleCloseResult,
};
use framer_core::resident_pool::{Acquire, ResidentStatus};
use std::collections::HashSet;
use std::time::Duration;
use tokio::{sync::Notify, time::timeout};

const WAIT: Duration = Duration::from_secs(3);
const CHANGED: &str = "account changed during deduplication; retry the preview";
const BUSY: &str = "account management is already in progress";
const READ_ERROR: &str = "fixture keeper read failed";
type Outcome = crate::pool_control::CleanupOutcome<Json<Value>>;

#[derive(Clone, Copy, PartialEq)]
enum Hold { None, Session, Remove }

struct SurvivorPorts {
    inner: Arc<FixturePorts>,
    keeper: AccountId,
    clone: AccountId,
    mutations: Arc<Mutex<HashSet<AccountId>>>,
    hold: Hold,
    fail_keeper_get: bool,
    entered: Notify,
    release: Notify,
    gets: Mutex<Vec<AccountId>>,
    removed: Mutex<Vec<AccountId>>,
    closed: Mutex<Vec<AccountId>>,
    guarded_io: Mutex<Vec<(AccountId, HashSet<AccountId>)>>,
}

impl SurvivorPorts {
    async fn pause(&self) -> Result<(), String> {
        self.entered.notify_one();
        timeout(WAIT, self.release.notified()).await
            .map_err(|_| "fixture release timed out".into())
    }

    fn record_guards(&self, id: AccountId) {
        let managed = self.mutations.lock().unwrap().clone();
        self.guarded_io.lock().unwrap().push((id, managed));
    }

    fn rows(&self) -> Value {
        serde_json::to_value(&*self.inner.records.lock().unwrap()).unwrap()
    }

    fn unexpected(&self, port: &str) -> ! { self.inner.unexpected(port) }
}

#[async_trait::async_trait]
impl StateStore for SurvivorPorts {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        self.gets.lock().unwrap().push(id);
        if self.fail_keeper_get && id == self.keeper { return Err(READ_ERROR.into()); }
        self.inner.get(id).await
    }

    async fn list(&self) -> Result<Vec<ManagedAccount>, String> { self.inner.list().await }

    async fn upsert(&self, _account: &ManagedAccount) -> Result<(), String> {
        self.unexpected("state")
    }

    async fn remove(&self, id: AccountId) -> Result<(), String> {
        if self.hold == Hold::Remove && id == self.clone { self.pause().await?; }
        self.record_guards(id);
        self.removed.lock().unwrap().push(id);
        self.inner.state_writes.fetch_add(1, Ordering::SeqCst);
        self.inner.records.lock().unwrap().retain(|row| row.id != id);
        Ok(())
    }
}

#[async_trait::async_trait]
impl SessionStore for SurvivorPorts {
    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        if self.hold == Hold::Session && user.0 == "user-1" { self.pause().await?; }
        self.inner.load(user).await
    }

    async fn save(&self, _session: &Session) -> Result<(), String> {
        self.unexpected("session")
    }

    async fn delete(&self, _user: &FramerUserId) -> Result<(), String> {
        self.unexpected("session")
    }
}

#[async_trait::async_trait]
impl BrowserBridge for SurvivorPorts {
    fn supports_browser_lifecycle(&self) -> bool { true }

    async fn castle_token(&self, _account: &AccountId) -> Result<CastleToken, String> {
        self.unexpected("browser")
    }

    async fn auth_begin(&self, _account: &AccountId, _email: &str) -> Result<(), String> {
        self.unexpected("browser")
    }

    async fn auth_complete(&self, _account: &AccountId, _token: &str)
        -> Result<(String, Value), String> {
        self.unexpected("browser")
    }

    async fn rotate_identity(&self, _account: &AccountId) -> Result<(), String> {
        self.unexpected("browser")
    }
    async fn browser_status(&self, id: &AccountId) -> Result<BrowserAccountStatus, String> {
        Ok(BrowserAccountStatus {
            account_id: *id, host_generation: Some("fixture-host".into()),
            account_incarnation: 1, active_commands: 0,
            state: if self.closed.lock().unwrap().contains(id) {
                BrowserAccountState::Absent
            } else { BrowserAccountState::Resident },
        })
    }

    async fn close_idle_browser(&self, id: &AccountId, expected: &BrowserAccountStatus)
        -> Result<IdleCloseResult, String> {
        self.record_guards(*id);
        self.closed.lock().unwrap().push(*id);
        let mut status = expected.clone();
        status.state = BrowserAccountState::Absent;
        Ok(IdleCloseResult { outcome: IdleCloseOutcome::Closed, status })
    }
}

fn survivor_fixture(hold: Hold, fail_keeper_get: bool)
    -> (AppState, Arc<SurvivorPorts>, ManagedAccount, ManagedAccount) {
    let (_, inner) = fixture(false);
    let keeper = account(1);
    let mut clone = account(2);
    clone.castle_device_id = keeper.castle_device_id.clone();
    clone.consecutive_failures = 1;
    *inner.records.lock().unwrap() = vec![clone.clone(), keeper.clone()];
    *inner.native_session.lock().unwrap() = Some(None);
    let serving = policy(&[1, 2]);
    let ports = Arc::new(SurvivorPorts {
        inner: inner.clone(), keeper: keeper.id, clone: clone.id,
        mutations: serving.mutations.clone(), hold, fail_keeper_get,
        entered: Notify::new(), release: Notify::new(),
        gets: Mutex::new(Vec::new()), removed: Mutex::new(Vec::new()),
        closed: Mutex::new(Vec::new()), guarded_io: Mutex::new(Vec::new()),
    });
    let state = AppState::fixture(
        inner, ports.clone(), ports.clone(), ports.clone(), serving,
        framer_core::gate::ConcurrencyGate::new(),
    );
    for id in [keeper.id, clone.id] {
        match state.serving_accounts.residents.try_acquire(id, None).unwrap() {
            Acquire::Lease(lease) => drop(lease),
            Acquire::Evict(_) => panic!("fixture unexpectedly needs eviction"),
        }
    }
    (state, ports, keeper, clone)
}

async fn apply(state: &AppState) -> Result<Outcome, tokio::time::error::Elapsed> {
    timeout(WAIT * 3, crate::dedupe_accounts_observed(
        State(state.clone()), Json(crate::DedupeBody { apply: true }),
    )).await
}

async fn remove(state: &AppState, id: AccountId)
    -> Result<Outcome, tokio::time::error::Elapsed> {
    timeout(WAIT, crate::remove_account_observed(
        State(state.clone()), Json(crate::RemoveBody { account_id: id, wipe_profile: false }),
    )).await
}

fn assert_busy(outcome: Outcome, expected: &str) {
    assert!(outcome.cleanup_confirmed);
    match outcome.result {
        Err(ApiError::PoolBusy(message)) => assert_eq!(message, expected),
        other => panic!("expected pool refusal, got {other:?}"),
    }
}

fn assert_settled(state: &AppState, ports: &SurvivorPorts, remaining: &[AccountId]) {
    assert!(state.serving_accounts.mutations.lock().unwrap().is_empty());
    let snapshot = state.serving_accounts.residents.snapshot().unwrap();
    assert_eq!(snapshot.occupied, remaining.len());
    assert_eq!(snapshot.residents.iter().map(|row| row.account).collect::<HashSet<_>>(),
        remaining.iter().copied().collect::<HashSet<_>>());
    assert!(snapshot.residents.iter().all(|row| row.users == 0
        && row.status == ResidentStatus::Idle && !row.uncertain
        && !row.close_started && row.replacement.is_none()));
    assert_eq!(state.serving_accounts.config(), policy(&[1, 2]).config());
    assert!(ports.inner.saved.lock().unwrap().is_empty());
    assert!(ports.inner.registered_proxies.lock().unwrap().is_empty());
    assert_eq!(ports.inner.browser_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.inner.client_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.inner.state_writes.load(Ordering::SeqCst),
        ports.removed.lock().unwrap().len());
}

#[tokio::test(flavor = "current_thread")]
async fn apply_refuses_clone_when_keeper_was_removed() {
    let (state, ports, keeper, clone) = survivor_fixture(Hold::Session, false);
    let intervention = async {
        let entered = timeout(WAIT, ports.entered.notified()).await.is_ok();
        let removal = if entered { Some(remove(&state, keeper.id).await) } else { None };
        ports.release.notify_one();
        (entered, removal)
    };
    let (dedupe, (entered, removal)) = tokio::join!(apply(&state), intervention);
    assert!(entered, "dedupe never reached the snapshot barrier");
    let removal = removal.unwrap().unwrap();
    assert!(removal.cleanup_confirmed);
    assert_eq!(removal.result.unwrap().0["status"], "removed");
    assert_busy(dedupe.unwrap(), CHANGED);
    assert_eq!(ports.rows(), serde_json::to_value(vec![clone.clone()]).unwrap());
    assert_eq!(*ports.gets.lock().unwrap(), vec![keeper.id, clone.id, keeper.id]);
    assert_eq!(*ports.removed.lock().unwrap(), vec![keeper.id]);
    assert_eq!(*ports.closed.lock().unwrap(), vec![keeper.id]);
    assert_eq!(*ports.guarded_io.lock().unwrap(),
        vec![(keeper.id, HashSet::from([keeper.id])); 2]);
    assert_settled(&state, &ports, &[clone.id]);
}

#[tokio::test(flavor = "current_thread")]
async fn apply_refuses_clone_when_keeper_device_changed() {
    let (state, ports, keeper, clone) = survivor_fixture(Hold::Session, false);
    let mut changed_keeper = keeper.clone();
    changed_keeper.castle_device_id = "fixture-replaced-device".into();
    let intervention = async {
        let entered = timeout(WAIT, ports.entered.notified()).await.is_ok();
        let changed = if entered {
            let mut rows = ports.inner.records.lock().unwrap();
            if let Some(row) = rows.iter_mut().find(|row| row.id == keeper.id) {
                row.castle_device_id = changed_keeper.castle_device_id.clone();
                true
            } else { false }
        } else { false };
        ports.release.notify_one();
        (entered, changed)
    };
    let (dedupe, (entered, changed)) = tokio::join!(apply(&state), intervention);
    assert!(entered && changed, "keeper drift was not installed at the barrier");
    assert_busy(dedupe.unwrap(), CHANGED);
    assert_eq!(ports.rows(), serde_json::to_value(vec![clone.clone(), changed_keeper]).unwrap());
    assert_eq!(*ports.gets.lock().unwrap(), vec![clone.id, keeper.id]);
    assert!(ports.removed.lock().unwrap().is_empty());
    assert!(ports.closed.lock().unwrap().is_empty());
    assert!(ports.guarded_io.lock().unwrap().is_empty());
    assert_settled(&state, &ports, &[keeper.id, clone.id]);
}

#[tokio::test(flavor = "current_thread")]
async fn apply_holds_keeper_until_clone_removal_settles() {
    let (state, ports, keeper, clone) = survivor_fixture(Hold::Remove, false);
    let intervention = async {
        let entered = timeout(WAIT, ports.entered.notified()).await.is_ok();
        let managed = ports.mutations.lock().unwrap().clone();
        let removal = if entered { Some(remove(&state, keeper.id).await) } else { None };
        ports.release.notify_one();
        (entered, managed, removal)
    };
    let (dedupe, (entered, managed, removal)) = tokio::join!(apply(&state), intervention);
    assert!(entered, "clone removal never reached its barrier");
    assert_eq!(managed, HashSet::from([keeper.id, clone.id]));
    assert_busy(removal.unwrap().unwrap(), BUSY);
    let dedupe = dedupe.unwrap();
    assert!(dedupe.cleanup_confirmed);
    let report = dedupe.result.unwrap().0;
    assert_eq!(report["removing"], json!([{
        "name": "fixture-2@example.invalid",
        "reason": "shares a Castle device id with another record"
    }]));
    assert_eq!(ports.rows(), serde_json::to_value(vec![keeper.clone()]).unwrap());
    assert_eq!(*ports.gets.lock().unwrap(), vec![clone.id, keeper.id]);
    assert_eq!(*ports.removed.lock().unwrap(), vec![clone.id]);
    assert_eq!(*ports.closed.lock().unwrap(), vec![clone.id]);
    assert_eq!(*ports.guarded_io.lock().unwrap(),
        vec![(clone.id, HashSet::from([keeper.id, clone.id])); 2]);
    assert_settled(&state, &ports, &[keeper.id]);
}

#[tokio::test(flavor = "current_thread")]
async fn apply_preserves_clone_on_keeper_read_or_management_error() {
    for busy in [false, true] {
        let (state, ports, keeper, clone) = survivor_fixture(Hold::None, !busy);
        let before = ports.rows();
        let holder = if busy {
            let held = timeout(WAIT, crate::pool_control::management_observed(
                &state, keeper.id, false,
            )).await.unwrap();
            assert!(held.cleanup_confirmed);
            Some(held.result.unwrap())
        } else { None };
        let outcome = apply(&state).await;
        drop(holder);
        let outcome = outcome.unwrap();
        if busy { assert_busy(outcome, BUSY); }
        else {
            assert!(outcome.cleanup_confirmed);
            match outcome.result {
                Err(ApiError::Internal(message)) => assert_eq!(message, READ_ERROR),
                other => panic!("expected original store error, got {other:?}"),
            }
        }
        assert_eq!(ports.rows(), before);
        assert_eq!(*ports.gets.lock().unwrap(), if busy {
            vec![clone.id]
        } else { vec![clone.id, keeper.id] });
        assert!(ports.removed.lock().unwrap().is_empty());
        assert!(ports.closed.lock().unwrap().is_empty());
        assert!(ports.guarded_io.lock().unwrap().is_empty());
        assert_settled(&state, &ports, &[keeper.id, clone.id]);
    }
}
