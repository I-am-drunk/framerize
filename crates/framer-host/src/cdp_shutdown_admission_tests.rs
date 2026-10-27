//! Local futures and held locks only. No preparation, process or peer starts.
use super::*;
use std::{task::Poll, time::Duration};
use tokio::time::timeout;

const LIMIT: Duration = Duration::from_secs(3);
const STOPPING: &str = "browser host is shutting down";
const UNCLEAN: &str = "could not confirm every owned browser exited cleanly";

#[derive(Clone, Copy)]
enum Caller { Instance, SeedSession, CastleToken, AiPost }

async fn call(bridge: &CdpBridge, account: &AccountId, caller: Caller)
    -> Result<(), String> {
    match caller {
        Caller::Instance => bridge.instance(account).await.map(|_| ()),
        Caller::SeedSession => bridge.seed_session(account, "fixture=value").await,
        Caller::CastleToken => bridge.castle_token(account).await.map(|_| ()),
        Caller::AiPost => bridge.ai_post(account, "/fixture", "fixture-token", &json!({}))
            .await.map(|_| ()),
    }
}

fn fixture(engine: EngineKind) -> (CdpBridge, PathBuf) {
    let root = std::env::temp_dir().join(format!("cdp-stop-admission-{}", uuid::Uuid::new_v4()));
    let bridge = CdpBridge::with_engine(root.join("unused"), root.join("profiles"), engine);
    (bridge, root)
}

fn remove_fixture(root: &Path) -> bool {
    match std::fs::remove_dir_all(root) {
        Ok(()) => true,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

#[derive(Debug)]
struct Observed {
    queued: bool,
    closing_pending: bool,
    stopping: bool,
    outcome: Option<Result<(), String>>,
    closed: Result<(), String>,
    route_state: Option<bool>,
    instances_empty: bool,
    startup_empty: bool,
    root_existed: bool,
    removed: bool,
}

async fn observe(engine: EngineKind, caller: Caller, uncertain: bool, cancel: bool) -> Observed {
    let (bridge, root) = fixture(engine);
    let account = AccountId::new();
    let cell = bridge.route_cell(&account).await;
    let mut route = cell.clone().lock_owned().await;
    route.may_have_browser = uncertain;
    let instances = bridge.instances.lock().await;
    let mut request = Box::pin(call(&bridge, &account, caller));
    let queued = futures::poll!(request.as_mut()).is_pending();
    let mut closing = Box::pin(bridge.shutdown());
    let closing_pending = futures::poll!(closing.as_mut()).is_pending();
    let stopping = bridge.stopping.load(Ordering::Acquire);
    let outcome = if cancel {
        drop(request);
        drop(route);
        None
    } else {
        drop(route);
        let polled = futures::poll!(request.as_mut());
        drop(request);
        match polled { Poll::Ready(result) => Some(result), Poll::Pending => None }
    };
    // On the baseline, the caller is blocked at this lock and is now dropped.
    // It cannot start a retained preparation task when the map is released.
    drop(instances);
    let closed = timeout(LIMIT, closing.as_mut()).await
        .unwrap_or_else(|_| Err("fixture shutdown timed out".into()));
    drop(closing);
    let route_state = cell.try_lock().ok().map(|route| route.may_have_browser);
    let instances_empty = bridge.instances.try_lock().is_ok_and(|map| map.is_empty());
    let startup_empty = timeout(LIMIT, bridge.startup_children.join()).await
        .is_ok_and(|outcomes| outcomes.is_empty());
    drop(bridge);
    let root_existed = root.exists();
    let removed = remove_fixture(&root);
    Observed { queued, closing_pending, stopping, outcome, closed, route_state,
        instances_empty, startup_empty, root_existed, removed }
}

fn assert_settled(observed: &Observed, uncertain: bool) {
    assert!(observed.queued && observed.closing_pending && observed.stopping);
    assert_eq!(observed.route_state, Some(uncertain));
    assert!(observed.instances_empty && observed.startup_empty);
    assert!(!observed.root_existed, "profile preparation escaped the fixture fence");
    assert!(observed.removed);
}

#[tokio::test]
async fn queued_instance_refuses_shutdown_before_cached_lookup() {
    for engine in [EngineKind::FullChrome, EngineKind::HeadlessShell, EngineKind::Attach] {
        let observed = observe(engine, Caller::Instance, false, false).await;
        assert_eq!(observed.outcome, Some(Err(STOPPING.into())));
        assert_settled(&observed, false);
        assert_eq!(observed.closed, Ok(()));
    }
}

#[tokio::test]
async fn queued_browser_callers_refuse_shutdown_before_downstream_work() {
    for caller in [Caller::SeedSession, Caller::CastleToken, Caller::AiPost] {
        let observed = observe(EngineKind::FullChrome, caller, false, false).await;
        assert_eq!(observed.outcome, Some(Err(STOPPING.into())));
        assert_settled(&observed, false);
        assert_eq!(observed.closed, Ok(()));
    }
}

#[tokio::test]
async fn queued_cleanup_uncertainty_keeps_shutdown_error_and_route_state() {
    for engine in [EngineKind::FullChrome, EngineKind::HeadlessShell] {
        let observed = observe(engine, Caller::Instance, true, false).await;
        assert_eq!(observed.outcome, Some(Err(STOPPING.into())));
        assert_settled(&observed, true);
        assert_eq!(observed.closed, Err(UNCLEAN.into()));
    }
}

#[tokio::test]
async fn cancelled_queued_caller_does_not_hold_up_shutdown() {
    for engine in [EngineKind::FullChrome, EngineKind::Attach] {
        let observed = observe(engine, Caller::Instance, false, true).await;
        assert_eq!(observed.outcome, None);
        assert_settled(&observed, false);
        assert_eq!(observed.closed, Ok(()));
    }
}

#[tokio::test]
async fn stopped_entry_preserves_early_error_and_attachment_owner() {
    let (bridge, root) = fixture(EngineKind::Attach);
    let owner = AccountId::new();
    let other = AccountId::new();
    let claimed = bridge.attached_owner.claim(owner);
    let closed = timeout(LIMIT, bridge.shutdown()).await;
    let result = timeout(LIMIT, call(&bridge, &other, Caller::Instance)).await;
    let retained = bridge.attached_owner.claim(owner);
    let collision = bridge.attached_owner.claim(other);
    let routes_empty = bridge.routes.try_lock().is_ok_and(|routes| routes.is_empty());
    drop(bridge);
    let root_existed = root.exists();
    let removed = remove_fixture(&root);
    claimed.unwrap();
    closed.unwrap().unwrap();
    assert_eq!(result.unwrap(), Err(STOPPING.into()));
    retained.unwrap();
    assert_eq!(collision, Err("attached browser is already assigned to another account".into()));
    assert!(routes_empty && !root_existed && removed);
}

#[tokio::test]
async fn active_entry_keeps_cached_lookup_and_cleanup_refusal_order() {
    for engine in [EngineKind::FullChrome, EngineKind::HeadlessShell, EngineKind::Attach] {
        let (bridge, root) = fixture(engine);
        let account = AccountId::new();
        let cell = bridge.route_cell(&account).await;
        let mut route = cell.clone().lock_owned().await;
        route.may_have_browser = true;
        let instances = bridge.instances.lock().await;
        let mut request = Box::pin(call(&bridge, &account, Caller::Instance));
        let queued = futures::poll!(request.as_mut()).is_pending();
        drop(route);
        let lookup_waited = futures::poll!(request.as_mut()).is_pending();
        drop(instances);
        let result = timeout(LIMIT, request.as_mut()).await;
        drop(request);
        let stopped_before_close = bridge.stopping.load(Ordering::Acquire);
        let closed = timeout(LIMIT, bridge.shutdown()).await;
        let route_state = cell.try_lock().ok().map(|route| route.may_have_browser);
        drop(bridge);
        let root_existed = root.exists();
        let removed = remove_fixture(&root);
        let attached = engine == EngineKind::Attach;
        let expected = if attached {
            "cdp-lifecycle-unconfirmed: browser cleanup is unconfirmed"
        } else { OWNED_STARTUP_UNCONFIRMED };
        assert!(queued && lookup_waited && !stopped_before_close);
        assert_eq!(result.unwrap(), Err(expected.into()));
        assert_eq!(closed.unwrap(), if attached { Ok(()) } else { Err(UNCLEAN.into()) });
        assert_eq!(route_state, Some(true));
        assert!(!root_existed && removed);
    }
}
