//! Synthetic prepare errors through the real coordinator and observer protocol.
use super::*;
use framer_api::experimental_native::native_account::CloseError;

// Extend the existing inert host before its entry point, retaining its exact
// context validation, fake browser close and framed observer writer.
const PREPARATION_FAILURE: &str = r#"
class PreparationHost(Host):
    async def pending_rpc(self):
        note('rpc_entered')
        await released('release_rpc')
        note('rpc_finished')

    async def handle(self, req):
        assert req['cmd'] == 'prepare_inference_authorization'
        assert req['purpose'] == 'baseline_context'
        await super().handle(req)
        scope = self.owner.scope_for(req)
        scope.comparison_next = 'failed'
        scope.rpc(self.pending_rpc())
        note('prepare_entered')
        await released('release_prepare')
        raise RuntimeError('fixture preparation failure')

Host = PreparationHost

class PreparationWriter(AsyncReplyWriter):
    async def send(self, frame):
        if frame.get('kind') == 'scope_cleanup':
            frame = dict(frame)
            mode = (TRACE / 'closure_mode').read_text()
            if mode == 'mismatch':
                frame['application_generation'] += 1
            elif mode == 'unconfirmed':
                frame['outcome'] = 'unconfirmed'
        await super().send(frame)
        if frame.get('ok') is False:
            note('prepare_error_sent')

AsyncReplyWriter = PreparationWriter

class PreparationOwner(Owner):
    def accept(self, req):
        if req['cmd'] in ('scope_stop', 'scope_seal', 'scope_forget'):
            with (TRACE / 'closure_commands').open('a') as out:
                out.write(req['cmd'] + '\n')
        super().accept(req)

Owner = PreparationOwner
"#;

fn preparation_pipe(mode: &str) -> RefusalPipe {
    let entry = "\nasyncio.run(main())";
    assert_eq!(REFUSAL_SIDECAR.matches(entry).count(), 1);
    let script = REFUSAL_SIDECAR.replacen(entry,
        &format!("{PREPARATION_FAILURE}{entry}"), 1);
    let pipe = RefusalPipe::new();
    std::fs::write(pipe._temp.0.join("fixture.py"), script).unwrap();
    std::fs::write(pipe.trace.join("closure_mode"), mode).unwrap();
    pipe
}

async fn failed_prepare_case(mode: &str, drop_waiter: bool) {
    let pipe = preparation_pipe(mode);
    let id = identity();
    let (store, _, _) = fixture(&id, None);
    let control = Arc::new(RefusalControl {
        identity: id.clone(), scope: Mutex::new(None), builds: AtomicUsize::new(0),
        bearers: AtomicUsize::new(0), authorizations: AtomicUsize::new(0),
    });
    let window = Arc::new(Window {
        selected: Mutex::new(Some(Selection {
            identity: id.clone(), context: context(), proxy: Some(FIXTURE_PROXY.into()),
        })),
        admissions: AtomicUsize::new(0), reports: Mutex::new(Vec::new()),
        recorded: Semaphore::new(0),
    });
    let slots = Arc::new(Semaphore::new(1));
    let outcome = std::panic::AssertUnwindSafe(async {
        let mut waiter = Some(pair::launch_with_control(spec(id), "baseline-turn".into(),
            pair::Dependencies { authority: window.clone(), store: store.clone(),
                observer: pipe.observer.as_ref().unwrap().clone(),
                permit: slots.clone().try_acquire_owned().unwrap() },
            Arc::new(RefusalFactory(control.clone()))).unwrap());
        pipe.event("prepare_entered").await;
        pipe.release("release_prepare");
        pipe.event("prepare_error_sent").await;
        pipe.event("close_entered").await;
        let scope = control.scope.lock().unwrap().as_ref().unwrap().clone();
        assert!(scope.closed().now_or_never().is_none(),
            "failed preparation retired before its browser close joined");
        assert!(window.reports.lock().unwrap().is_empty());
        assert_eq!(slots.available_permits(), 0);
        assert_eq!(control.bearers.load(Ordering::SeqCst), 0);
        assert_eq!(control.authorizations.load(Ordering::SeqCst), 0);
        assert!(waiter.as_mut().unwrap().as_mut().now_or_never().is_none());
        if drop_waiter { drop(waiter.take()); }
        pipe.release("release_close");
        pipe.event("browser_closed").await;
        pipe.event("rpc_entered").await;
        assert!(scope.closed().now_or_never().is_none(),
            "browser close acknowledgement did not settle the retained RPC");
        assert!(window.reports.lock().unwrap().is_empty());
        assert_eq!(slots.available_permits(), 0);
        pipe.release("release_rpc");
        signalled(&window.recorded).await;
        let matched = mode == "matching";
        let failure = if matched { Failure::Source } else { Failure::Cleanup };
        if let Some(waiter) = waiter {
            let report = tokio::time::timeout(BOUND, waiter).await.unwrap();
            assert_eq!(report.failure, Some(failure));
            assert_eq!(report.account_closed, matched);
            assert_eq!(report.browser_cleanup_confirmed, matched);
            assert!(report.baseline.is_none() && report.native.is_none());
            assert!(!report.native_nonempty_text);
        }
        assert_eq!(*window.reports.lock().unwrap(), vec![Outcome {
            failure: Some(failure), baseline: false, native: false,
            browser_cleanup: matched, account_closed: matched,
        }]);
        assert_eq!(scope.closed().now_or_never(), Some(
            if matched { Ok(()) } else { Err(CloseError::PreparationUnconfirmed) }));
        assert_eq!(window.admissions.load(Ordering::SeqCst), 1);
        assert_eq!(store.loads.load(Ordering::SeqCst), 1);
        assert_eq!(control.builds.load(Ordering::SeqCst), 1);
        assert_eq!(control.bearers.load(Ordering::SeqCst), 0);
        assert_eq!(control.authorizations.load(Ordering::SeqCst), 0);
        assert_eq!(slots.available_permits(), 1);
        pipe.event("rpc_finished").await;
        if matched { pipe.event("scope_forgotten").await; }
        else { assert!(!pipe.trace.join("scope_forgotten").exists()); }
        assert_eq!(std::fs::read_to_string(pipe.trace.join("commands")).unwrap(),
            "prepare_inference_authorization:baseline_context\n");
        assert_eq!(std::fs::read_to_string(pipe.trace.join("closure_commands")).unwrap(),
            if matched { "scope_stop\nscope_forget\n" } else { "scope_stop\n" });
    }).catch_unwind().await;
    pipe.release("release_prepare");
    pipe.release("release_rpc");
    pipe.finish().await;
    if let Err(panic) = outcome { std::panic::resume_unwind(panic); }
}

#[tokio::test]
async fn prepare_error_waits_for_matching_browser_close_before_account_close() {
    failed_prepare_case("matching", false).await;
}

#[tokio::test]
async fn prepare_error_mismatched_closure_retains_native_fence() {
    failed_prepare_case("mismatch", false).await;
}

#[tokio::test]
async fn prepare_error_unconfirmed_closure_retains_native_fence() {
    failed_prepare_case("unconfirmed", false).await;
}

#[tokio::test]
async fn dropped_prepare_result_waiter_retains_close_and_publication() {
    failed_prepare_case("matching", true).await;
}
