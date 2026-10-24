//! Inert coordinator ownership checks. No browser, bearer or inference I/O.
use super::*;
use framer_host::sidecar::observer::{ApplicationGeneration, CleanupOutcome, ObserverError};
use std::time::Duration;

const BOUND: Duration = Duration::from_secs(3);

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    failure: Option<Failure>,
    baseline: bool,
    native: bool,
    browser_cleanup: bool,
    account_closed: bool,
}

struct Window {
    selected: Mutex<Option<Selection>>,
    admissions: AtomicUsize,
    reports: Mutex<Vec<Outcome>>,
    recorded: Semaphore,
}

impl pair::ComparisonWindow for Window {
    fn admit(&self) -> FutureResult<'_, Result<Selection, Failure>> {
        self.admissions.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(self.selected.lock().unwrap().take().expect("one admission")) })
    }
    fn record(&self, report: &pair::Report) -> FutureResult<'_, Result<(), Failure>> {
        self.reports.lock().unwrap().push(Outcome {
            failure: report.failure, baseline: report.baseline.is_some(),
            native: report.native.is_some(), browser_cleanup: report.browser_cleanup_confirmed,
            account_closed: report.account_closed,
        });
        self.recorded.add_permits(1);
        Box::pin(async { Ok(()) })
    }
}

struct HeldStore {
    inner: Arc<Store>,
    entered: Semaphore,
    release: Semaphore,
}

impl SessionStore for HeldStore {
    fn load<'a, 'b, 'f>(&'a self, user: &'b FramerUserId)
        -> FutureResult<'f, Result<Option<Session>, String>> where 'a: 'f, 'b: 'f, Self: 'f {
        Box::pin(async move {
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            self.inner.load(user).await
        })
    }
    fn save<'a, 'b, 'f>(&'a self, session: &'b Session)
        -> FutureResult<'f, Result<(), String>> where 'a: 'f, 'b: 'f, Self: 'f {
        self.inner.save(session)
    }
    fn delete<'a, 'b, 'f>(&'a self, user: &'b FramerUserId)
        -> FutureResult<'f, Result<(), String>> where 'a: 'f, 'b: 'f, Self: 'f {
        self.inner.delete(user)
    }
}

async fn signalled(signal: &Semaphore) {
    tokio::time::timeout(BOUND, signal.acquire()).await
        .expect("coordinator did not reach fixture barrier").unwrap().forget();
}

#[tokio::test]
async fn dropped_comparison_waiter_still_closes_account_and_records_scope_refusal() {
    let (_root, observer) = observer();
    let id = identity();
    let (store, _, _) = fixture(&id, None);
    let store = Arc::new(HeldStore {
        inner: store, entered: Semaphore::new(0), release: Semaphore::new(0),
    });
    // Occupy only the observer slot. The coordinator must still close its
    // newly opened native account after this definite, pre-command refusal.
    let held = observer.one_shot_scope(id.account, ApplicationGeneration(71)).unwrap();
    let window = Arc::new(Window {
        selected: Mutex::new(Some(Selection {
            identity: id.clone(), context: context(), proxy: None,
        })),
        admissions: AtomicUsize::new(0), reports: Mutex::new(Vec::new()),
        recorded: Semaphore::new(0),
    });
    let slots = Arc::new(Semaphore::new(1));
    let waiter = pair::launch(spec(id.clone()), "baseline-turn".into(), pair::Dependencies {
        authority: window.clone(), store: store.clone(), observer: observer.clone(),
        permit: slots.clone().try_acquire_owned().unwrap(),
    }).unwrap();
    signalled(&store.entered).await;
    drop(waiter);
    tokio::task::yield_now().await;
    assert_eq!(slots.available_permits(), 0, "owner released its permit while load was pending");
    assert!(window.reports.lock().unwrap().is_empty());
    assert_eq!(store.inner.loads.load(Ordering::SeqCst), 0);
    store.release.add_permits(1);
    signalled(&window.recorded).await;
    assert_eq!(window.admissions.load(Ordering::SeqCst), 1);
    assert_eq!(store.inner.loads.load(Ordering::SeqCst), 1);
    assert_eq!(slots.available_permits(), 1);
    assert_eq!(*window.reports.lock().unwrap(), vec![Outcome {
        failure: Some(Failure::Source), baseline: false, native: false,
        browser_cleanup: false, account_closed: true,
    }]);
    assert!(matches!(observer.one_shot_scope(id.account, ApplicationGeneration(72)),
        Err(ObserverError::AccountBusy)), "coordinator retired somebody else's scope");
    held.control.seal_for_account_teardown();
    assert_eq!(tokio::time::timeout(BOUND, held.cleanup.wait()).await.unwrap(),
        CleanupOutcome::NotStarted { reason: None });
}
