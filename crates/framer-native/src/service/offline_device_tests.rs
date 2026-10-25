use super::*;

pub(super) struct DeviceBuilder(pub Arc<Ports>);
impl SourceBuilder for DeviceBuilder {
    fn build(&self, _: AccountScope, _: Option<String>)
        -> Result<Arc<dyn CredentialSource>, Failure> { Err(Failure::MissingAuthority) }
    fn device_owner(&self) -> Option<Arc<dyn DeviceOwner>> { Some(self.0.clone()) }
    fn build_with_device(&self, scope: AccountScope, proxy: Option<String>,
        device: Option<Arc<crate::account::device::DeviceGrant>>)
        -> Result<Arc<dyn CredentialSource>, Failure> {
        let device = device.expect("shared host grant required before source construction");
        assert_eq!(device.account, scope.account());
        assert_eq!(device.device, "fixture-device");
        assert_eq!(self.0.device_users.load(Ordering::SeqCst), 1);
        Builder(self.0.clone()).build(scope, proxy)
    }
}

fn start_device(state: ServiceState, ports: &Arc<Ports>, spec: Specification,
    sessions: &Sessions) -> Run {
    launch_with_source(state, spec, Arc::new(DeviceBuilder(ports.clone())), sessions.clone()).unwrap()
}

#[tokio::test]
async fn device_ownership_survives_cancellation_through_scope_and_host_cleanup() {
    let (state, ports, spec, sessions) = fixture();
    ports.issue.try_acquire().unwrap().forget();
    ports.device_finish.try_acquire().unwrap().forget();
    let run = start_device(state, &ports, spec, &sessions);
    wait(&ports.issuing).await;
    run.cancel();
    assert_eq!(ports.device_users.load(Ordering::SeqCst), 1);
    assert!(run.cleanup.clone().now_or_never().is_none());
    ports.issue.add_permits(1);
    wait(&ports.device_closing).await;
    assert_eq!(ports.device_users.load(Ordering::SeqCst), 1);
    assert!(run.cleanup.clone().now_or_never().is_none());
    ports.device_finish.add_permits(1);
    let outcome = finish(run).await;
    assert_eq!(outcome.failure, Some(FailureKind::Cancelled));
    assert!(outcome.cleanup_confirmed);
    assert_eq!(ports.device_users.load(Ordering::SeqCst), 0);
    assert_eq!(*ports.events.lock().unwrap(),
        vec!["device_open", "load", "loaded", "device_close"]);
}

#[tokio::test]
async fn device_rejection_and_lost_open_are_distinct_before_private_io() {
    for mode in [1, 2] {
        let (state, ports, spec, sessions) = fixture();
        ports.device_mode.store(mode, Ordering::SeqCst);
        let outcome = finish(start_device(state, &ports, spec, &sessions)).await;
        assert_eq!(*ports.events.lock().unwrap(), vec!["device_open"]);
        assert_eq!(outcome.cleanup_confirmed, mode == 1);
        assert_eq!(outcome.failure, Some(if mode == 1 { FailureKind::Ineligible }
            else { FailureKind::Cleanup }));
    }
}

#[tokio::test]
async fn device_loss_keeps_the_original_account_fenced() {
    for mode in [0, 2, 3] {
        let (state, ports, spec, sessions) = fixture();
        let identity = spec.identity.clone();
        ports.uncertain.store(mode == 0, Ordering::SeqCst);
        ports.device_mode.store(mode, Ordering::SeqCst);
        let outcome = finish(start_device(state.clone(), &ports, spec, &sessions)).await;
        assert_eq!(outcome.failure, Some(FailureKind::Cleanup));
        assert!(!outcome.cleanup_confirmed && outcome.response.is_none());
        assert_eq!(ports.device_users.load(Ordering::SeqCst), 1);
        assert!(native_control::admit(&state, identity, "gpt-4.1",
            Instant::now() + Duration::from_secs(1)).await.is_err());
    }
}
