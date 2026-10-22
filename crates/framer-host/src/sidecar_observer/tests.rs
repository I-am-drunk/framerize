use super::*;

fn limits() -> ObserverLimits {
    ObserverLimits { scopes: 2, commands: 3, cleanup_timeout: Duration::from_secs(1) }
}

fn isolated() -> Arc<SidecarBridge> {
    let mut bridge = SidecarBridge::new("unopened-observer-fixture");
    bridge.mode = ProcessMode::Observed {
        script: PathBuf::from("must-not-be-launched"), profile_nonce: Uuid::new_v4(),
    };
    Arc::new(bridge)
}

pub(super) fn local_state() -> (Arc<state::ScopeState>, Arc<Registry>) {
    let registry = Arc::new(Registry::new(limits()));
    let scope = Arc::new(state::ScopeState {
        _owner: isolated(), account: AccountId(Uuid::new_v4()),
        application: ApplicationGeneration(19), id: Uuid::new_v4(),
        phase: StdMutex::new(state::Phase::default()), changed: Notify::new(),
        proxy: StdMutex::new(None), queue_wait: AtomicU64::new(0),
        _slot: registry.slots.clone().try_acquire_owned().unwrap(),
    });
    (scope, registry)
}

#[test]
fn unmanaged_bridge_cannot_be_adopted() {
    assert!(matches!(Arc::new(SidecarBridge::new("unused")).observer_factory(limits()),
        Err(ObserverError::Unsupported)));
}

#[test]
fn limits_reserve_control_capacity_inside_512_records() {
    let mut config = limits(); config.commands = 510;
    assert!(matches!(isolated().observer_factory(config), Err(ObserverError::InvalidLimits)));
}

#[test]
fn factory_configuration_is_immutable() {
    let base = isolated();
    base.observer_factory(limits()).unwrap();
    let mut config = limits(); config.commands = 4;
    assert!(matches!(base.observer_factory(config), Err(ObserverError::LimitsConflict)));
}

#[tokio::test]
async fn unused_scope_seals_without_a_process() {
    let base = isolated();
    let factory = base.observer_factory(limits()).unwrap();
    let scope = factory.one_shot_scope(AccountId(Uuid::new_v4()), ApplicationGeneration(1)).unwrap();
    scope.control.seal_for_account_teardown();
    assert_eq!(tokio::time::timeout(Duration::from_secs(1), scope.cleanup.wait()).await.unwrap(),
        CleanupOutcome::NotStarted { reason: None });
    assert!(base.inner.lock().await.is_none());
}

#[test]
fn lost_worker_retains_uncertainty_after_late_success() {
    let (state, registry) = local_state();
    drop(state.begin(&registry).unwrap());
    state.publish(CleanupOutcome::NotStarted { reason: None });
    let phase = state.phase.lock().unwrap();
    assert_eq!(phase.active, 0);
    assert_eq!(phase.outcome, Some(CleanupOutcome::Unconfirmed(ObserverError::WorkerLost)));
}

#[test]
fn completing_command_releases_capacity_without_claiming_cleanup() {
    let (state, registry) = local_state();
    state.begin(&registry).unwrap().complete();
    let phase = state.phase.lock().unwrap();
    assert_eq!(phase.active, 0);
    assert_eq!(registry.commands.available_permits(), limits().commands);
    assert!(phase.outcome.is_none());
}

#[test]
fn seal_excludes_new_commands_but_keeps_existing_owner() {
    let (state, registry) = local_state();
    let command = state.begin(&registry).unwrap();
    state.seal();
    assert!(matches!(state.begin(&registry), Err(ObserverError::Sealed)));
    assert_eq!(state.phase.lock().unwrap().active, 1);
    command.complete();
}

#[tokio::test]
async fn other_account_is_rejected_before_cookie_parse_or_admission() {
    let (state, registry) = local_state();
    let bridge = bridge::ScopedBridge { state: state.clone(), registry };
    let error = bridge.seed_session(&AccountId(Uuid::new_v4()), "invalid").await.unwrap_err();
    assert_eq!(error, ObserverError::AccountMismatch.to_string());
    assert!(!state.phase.lock().unwrap().started);
}

#[tokio::test]
async fn proxy_change_after_admission_is_explicitly_unsupported() {
    let (state, registry) = local_state();
    let bridge = bridge::ScopedBridge { state: state.clone(), registry: registry.clone() };
    bridge.set_proxy(&state.account, Some("http://fixture.invalid".into())).await.unwrap();
    let command = state.begin(&registry).unwrap();
    assert_eq!(bridge.set_proxy(&state.account, None).await.unwrap_err(),
        ObserverError::UnsupportedMutation.to_string());
    command.complete();
}

#[test]
fn repeated_stop_keeps_one_flag_and_existing_command() {
    let (state, registry) = local_state();
    let command = state.begin(&registry).unwrap();
    for _ in 0..1000 { state.stop(); }
    let p = state.phase.lock().unwrap();
    assert!(p.stop && p.sealed);
    assert_eq!(p.active, 1);
    assert!(p.outcome.is_none());
    drop(p);
    command.complete();
}

#[test]
fn command_capacity_does_not_evict_existing_records() {
    let (state, registry) = local_state();
    let commands: Vec<_> = (0..limits().commands).map(|_| state.begin(&registry).unwrap()).collect();
    assert!(matches!(state.begin(&registry), Err(ObserverError::Capacity)));
    assert_eq!(state.phase.lock().unwrap().active, limits().commands);
    for command in commands { command.complete(); }
}

fn terminal() -> (Uuid, Value) {
    let incarnation = Uuid::new_v4();
    (incarnation, json!({"kind": "scope_cleanup", "protocol": 1,
        "incarnation": incarnation.to_string(), "account": Uuid::new_v4().to_string(),
        "scope": Uuid::new_v4().to_string(), "application_generation": 37,
        "finish": Uuid::new_v4().to_string(), "extent": "account_browser_closed", "outcome": "closed"}))
}

#[test]
fn terminal_preserves_all_registered_identity_fields() {
    let (incarnation, v) = terminal();
    match RemoteEvent::parse(&v, incarnation).unwrap() {
        RemoteEvent::Cleanup { account, scope, generation, finish, clean } => {
            assert_eq!(v["account"], account.0.to_string());
            assert_eq!(v["scope"], scope.to_string());
            assert_eq!(generation, ApplicationGeneration(37));
            assert_eq!(v["finish"], finish.to_string());
            assert!(clean);
        },
        _ => panic!("valid terminal was not parsed"),
    }
}

#[test]
fn replacement_incarnation_cannot_complete_original_scope() {
    let (_, v) = terminal();
    assert!(matches!(RemoteEvent::parse(&v, Uuid::new_v4()), Some(RemoteEvent::Lost)));
}

#[test]
fn missing_identity_or_unknown_cleanup_extent_is_not_clean() {
    let (incarnation, original) = terminal();
    for key in ["account", "scope", "application_generation", "finish", "extent", "protocol"] {
        let mut v = original.clone();
        v.as_object_mut().unwrap().remove(key);
        assert!(matches!(RemoteEvent::parse(&v, incarnation), Some(RemoteEvent::Lost)));
    }
}

#[test]
fn ordinary_result_is_not_consumed_as_cleanup() {
    assert!(RemoteEvent::parse(&json!({"id": 1, "ok": true, "result": "value"}), Uuid::new_v4()).is_none());
}

#[test]
fn repeated_stop_does_not_extend_the_cleanup_deadline() {
    let (state, _) = local_state();
    state.seal();
    let at = state.phase.lock().unwrap().sealed_at.unwrap();
    for _ in 0..1000 { state.stop(); }
    assert_eq!(state.phase.lock().unwrap().sealed_at, Some(at));
}

#[test]
fn receipt_without_registered_incarnation_is_not_accepted() {
    let (state, _) = local_state();
    let receipt = CleanupReceipt { account: state.account,
        application_generation: state.application, scope: state.id,
        sidecar_incarnation: Uuid::new_v4(), extent: CleanupExtent::AccountBrowserClosed };
    state.publish(CleanupOutcome::Closed(receipt.clone()));
    let control = scope::ScopeControl { state };
    assert!(!control.matches_closed_receipt(&receipt));
}
