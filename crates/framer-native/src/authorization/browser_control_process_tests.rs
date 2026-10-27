use super::*;
use crate::account::client::{NativeAccountClient, PendingPreparation};
use crate::account::device::DeviceAuthority;
use crate::authorization::control::browser_control_fixture::{target, fixture};
use std::sync::Arc;

struct Case {
    client: NativeAccountClient,
    scope: AccountScope,
    peer: Arc<BrowserControlProcess>,
    grant: Arc<DeviceGrant>,
    directory: PathBuf,
}

fn sha(path: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}

fn case(mode: &str) -> Case {
    let (client, scope) = fixture::scope(crate::transport::http::NativeEgress::Direct);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let script = root.join("scripts/experiments/native_browser_handoff/control_protocol_fixture.py");
    let python = PathBuf::from(std::env::var_os("FRAMERIZE_TEST_CONTROL_PYTHON")
        .expect("an explicitly bound Python interpreter is required"));
    let expected = std::env::var("FRAMERIZE_TEST_CONTROL_PYTHON_SHA256").unwrap();
    assert_eq!(sha(&python), expected);
    let directory = std::env::temp_dir().join(format!("control-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let launch = BrowserControlLaunch { python, python_sha256: expected,
        script_sha256: sha(&script), script,
        arguments: vec![mode.into(), directory.as_os_str().to_owned()] };
    let grant = Arc::new(DeviceGrant { account: scope.account(),
        host_generation: "a".repeat(32), lease: uuid::Uuid::new_v4(),
        device: "fixture-device".into(), authority: DeviceAuthority::Ledger });
    let binding = BrowserControlBinding { account: scope.account(),
        user: scope.active_user().clone(), session: "fixture-only-cookie".into(),
        device: grant.device.clone(), page_url: target().page_url().into(),
        wrapper_url: "https://app.framerstatic.com/fixture-wrapper.mjs".into(),
        post_path: target().path().into(), proxy: None };
    let mut peer = BrowserControlProcess::new(launch, binding).unwrap();
    peer.prepare_limit = Duration::from_secs(5);
    peer.cleanup_limit = Duration::from_secs(2);
    Case { client, scope, peer: Arc::new(peer), grant, directory }
}

impl Drop for Case {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.directory); }
}

impl Case {
    fn mint(&self, budget: u64) -> PendingPreparation<(String, Duration)> {
        let peer = self.peer.clone();
        let scope = self.scope.clone();
        let grant = self.grant.clone();
        self.client.prepare_owned(&self.scope, Instant::now() + Duration::from_secs(5),
            move |session, control| async move {
                let result = peer.mint(&scope, &session, &grant, &target(), budget, &control).await;
                PreparationResult { cleanup_confirmed: result.cleanup_confirmed,
                    value: result.value.map(|m| (m.token.0, m.received_at.elapsed())) }
            }).unwrap()
    }

    async fn marker(&self, name: &str) {
        timeout(Duration::from_secs(3), async {
            while !self.directory.join(name).exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.unwrap();
    }

    fn release(&self, name: &str) {
        std::fs::write(self.directory.join(name), b"release").unwrap();
    }

    fn reaped(&self) {
        let pid: i32 = std::fs::read_to_string(self.directory.join("pid"))
            .unwrap().parse().unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }

    async fn close(&self) {
        self.peer.shutdown().await;
        if let Ok(observer) = self.client.close_account(&self.scope) {
            let _ = timeout(Duration::from_secs(3), observer).await.unwrap();
        }
    }
}

#[tokio::test]
async fn private_child_holds_control_until_physical_cleanup() {
    let case = case("hold-close");
    assert!(!case.directory.join("pid").exists());
    assert!(case.peer.prepare().await);
    assert_eq!(case.mint(2000).await.unwrap().0, "fixture-token");
    assert!(!case.directory.join("physically-closed").exists());
    let finished = case.peer.finish(&case.grant, true, false);
    tokio::pin!(finished);
    tokio::select! {
        _ = &mut finished => panic!("finish returned before physical close"),
        _ = case.marker("close-entered") => {}
    }
    assert!(finished.as_mut().now_or_never().is_none());
    case.release("close-release");
    assert!(finished.await);
    case.reaped();
    case.close().await;
}

#[tokio::test]
async fn cancelled_prepare_reaps_child_without_confirming_cleanup() {
    let case = case("hold-prepare");
    {
        let preparation = case.peer.prepare();
        tokio::pin!(preparation);
        tokio::select! {
            _ = &mut preparation => panic!("prepare escaped its fixture hold"),
            _ = case.marker("prepare-entered") => {}
        }
    }
    assert!(!case.peer.shutdown().await);
    case.reaped();
    assert!(!case.peer.prepare().await);
    case.close().await;
}

#[tokio::test]
async fn unstarted_shutdown_does_not_launch_child() {
    let case = case("normal");
    assert!(case.peer.shutdown().await);
    assert!(case.peer.shutdown().await);
    assert!(!case.peer.prepare().await);
    assert!(!case.directory.join("pid").exists());
    case.close().await;
}

#[tokio::test]
async fn cancellation_preserves_partial_frame_until_joined_closed_reply() {
    let case = case("partial-mint");
    assert!(case.peer.prepare().await);
    let pending = case.mint(2000);
    let completion = pending.completion();
    case.marker("partial-entered").await;
    pending.cancellation().cancel();
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(completion.clone().now_or_never().is_none());
    case.release("partial-release");
    assert_eq!(pending.await, Err(PreparationFailure::Cancelled));
    completion.await.unwrap();
    assert!(case.peer.finish(&case.grant, true, false).await);
    case.reaped();
    case.close().await;
}

#[tokio::test]
async fn preparation_deadline_and_false_egress_refuse_and_reap() {
    for mode in ["hold-prepare", "bad-egress"] {
        let mut case = case(mode);
        Arc::get_mut(&mut case.peer).unwrap().prepare_limit = Duration::from_secs(2);
        assert!(!case.peer.prepare().await, "{mode}");
        case.reaped();
        assert!(!case.peer.shutdown().await);
        assert!(!case.peer.prepare().await);
        case.close().await;
    }
}

#[tokio::test]
async fn one_use_peer_retains_owner_after_rejected_second_mint_or_finish() {
    let case = case("normal");
    assert!(case.peer.prepare().await);
    case.mint(2000).await.unwrap();
    assert_eq!(case.mint(2000).await, Err(PreparationFailure::Rejected));
    let other = DeviceGrant { account: case.grant.account,
        host_generation: case.grant.host_generation.clone(),
        lease: uuid::Uuid::new_v4(), device: case.grant.device.clone(),
        authority: DeviceAuthority::Ledger };
    assert!(!case.peer.finish(&other, true, false).await);
    assert!(!case.directory.join("physically-closed").exists());
    assert!(case.peer.finish(&case.grant, true, true).await);
    case.reaped();
    assert!(!case.peer.prepare().await);
    case.close().await;
}

#[tokio::test]
async fn unjoined_request_never_confirms_even_when_child_closes() {
    let case = case("normal");
    assert!(case.peer.prepare().await);
    case.mint(2000).await.unwrap();
    assert!(!case.peer.finish(&case.grant, false, false).await);
    assert!(!case.peer.shutdown().await);
    case.reaped();
    case.close().await;
}

#[tokio::test]
async fn changed_context_refuses_mint_without_consuming_peer() {
    for changed in 0..7 {
        let mut case = case("normal");
        let binding = &mut Arc::get_mut(&mut case.peer).unwrap().binding;
        match changed {
            0 => binding.account = framer_core::ids::AccountId(uuid::Uuid::new_v4()),
            1 => binding.user.0 = "another-user".into(),
            2 => binding.session = "another-cookie".into(),
            3 => binding.device = "another-device".into(),
            4 => binding.page_url.push_str("-other"),
            5 => binding.post_path = "/ai/v3/openai/responses/".into(),
            _ => binding.proxy = Some("http://127.0.0.1:9".into()),
        }
        assert!(case.peer.prepare().await, "context {changed}");
        assert_eq!(case.mint(2000).await, Err(PreparationFailure::Rejected), "context {changed}");
        assert!(case.peer.state.lock().await.grant.is_none());
        assert!(case.peer.shutdown().await, "context {changed}");
        case.reaped();
        case.close().await;
    }
}

#[tokio::test]
async fn malformed_or_mismatched_mint_reply_is_unconfirmed_and_reaped() {
    for mode in ["wrong-lease", "oversized", "duplicate"] {
        let case = case(mode);
        assert!(case.peer.prepare().await, "{mode}");
        assert_eq!(case.mint(2000).await, Err(PreparationFailure::CleanupUnconfirmed), "{mode}");
        assert!(!case.peer.shutdown().await, "{mode}");
        case.reaped();
        case.close().await;
    }
}

#[tokio::test]
async fn trailing_output_or_failed_exit_cannot_confirm_cleanup() {
    for mode in ["extra-frame", "bad-exit"] {
        let case = case(mode);
        assert!(case.peer.prepare().await, "{mode}");
        case.mint(2000).await.unwrap();
        assert!(!case.peer.finish(&case.grant, true, false).await, "{mode}");
        assert!(!case.peer.shutdown().await, "{mode}");
        case.reaped();
        case.close().await;
    }
}

#[tokio::test]
async fn changed_executable_or_script_pin_prevents_spawn() {
    for executable in [true, false] {
        let mut case = case("normal");
        let peer = Arc::get_mut(&mut case.peer).unwrap();
        if executable { peer.launch.python_sha256 = "0".repeat(64); }
        else { peer.launch.script_sha256 = "0".repeat(64); }
        assert!(!case.peer.prepare().await);
        assert!(!case.directory.join("pid").exists());
        assert!(!case.peer.shutdown().await);
        case.close().await;
    }
}

#[tokio::test]
async fn mint_deadline_still_joins_rpc_before_releasing_child() {
    let case = case("hold-mint");
    assert!(case.peer.prepare().await);
    let pending = case.mint(150);
    let completion = pending.completion();
    case.marker("mint-entered").await;
    tokio::time::sleep(Duration::from_millis(180)).await;
    assert!(completion.clone().now_or_never().is_none());
    assert!(!case.directory.join("physically-closed").exists());
    case.release("mint-release");
    assert_eq!(pending.await, Err(PreparationFailure::Deadline));
    completion.await.unwrap();
    assert!(case.peer.finish(&case.grant, true, false).await);
    case.reaped();
    case.close().await;
}

#[tokio::test]
async fn closed_acknowledgement_still_waits_for_process_exit() {
    let case = case("hold-exit");
    assert!(case.peer.prepare().await);
    case.mint(2000).await.unwrap();
    let finished = case.peer.finish(&case.grant, true, false);
    tokio::pin!(finished);
    tokio::select! {
        _ = &mut finished => panic!("finish returned before child exit"),
        _ = case.marker("exit-entered") => {}
    }
    assert_eq!(std::fs::read_to_string(case.directory.join("physically-closed")).unwrap(), "True");
    assert!(finished.as_mut().now_or_never().is_none());
    case.release("exit-release");
    assert!(finished.await);
    case.reaped();
    case.close().await;
}

#[tokio::test]
async fn cancelled_mint_waits_for_child_rpc_and_suppresses_token() {
    let case = case("hold-mint");
    assert!(case.peer.prepare().await);
    let pending = case.mint(2000);
    let completion = pending.completion();
    case.marker("mint-entered").await;
    pending.cancellation().cancel();
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(completion.clone().now_or_never().is_none());
    assert!(!case.directory.join("physically-closed").exists());
    case.release("mint-release");
    assert_eq!(pending.await, Err(PreparationFailure::Cancelled));
    completion.await.unwrap();
    assert!(case.peer.finish(&case.grant, true, false).await);
    case.reaped();
    case.close().await;
}
