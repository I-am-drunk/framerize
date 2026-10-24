//! Inert coordinator ownership checks. No browser, bearer or inference I/O.
use super::*;
use framer_host::sidecar::observer::{ApplicationGeneration, CleanupOutcome, ObserverError};
use std::time::Duration;
use framer_api::experimental_native::authorization::{CredentialTarget, PreparedCredentials};
use framer_core::port::BrowserAuthorization;
use futures::FutureExt;

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

const FIXTURE_PROXY: &str = "http://127.0.0.1:9";
const CURRENT_COOKIES: &str = "session=fixture-secret; __cuid=fixture-device";

fn assert_refusal(report: &framer_host::sidecar::observer::comparison::BaselineReport) {
    use framer_host::sidecar::observer::comparison::{ContentType, RefusalBasis, RefusalPolicy};
    assert!(!report.accepted);
    assert_eq!(report.status, Some(403));
    assert_eq!(report.content_type, ContentType::Json);
    assert!(report.response_complete && report.fetch_settled);
    assert!(!report.complete_json && !report.nonempty_text);
    assert_eq!(report.body_bytes, 231);
    assert_eq!((report.transport_starts, report.transport_finishes), (Some(1), Some(1)));
    let refusal = report.refusal.unwrap();
    assert_eq!(refusal.policy, RefusalPolicy::Castle);
    assert_eq!(refusal.basis, RefusalBasis::FramerSecurityEnvelope);
    let public = format!("{report:?}");
    for private in ["fixture-private", "fixture-bearer", "fixture-secret", "fixture-device"] {
        assert!(!public.contains(private));
    }
}

struct RefusalControl {
    identity: Identity,
    scope: Mutex<Option<AccountScope>>,
    builds: AtomicUsize,
    bearers: AtomicUsize,
    authorizations: AtomicUsize,
}
struct RefusalFactory(Arc<RefusalControl>);

impl pair::control::Control for RefusalControl {
    fn bearer<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        target: &'a CredentialTarget, page: &'a str, cookies: &'a str,
        cancel: &'a PreparationControl) -> FutureResult<'a, PreparationResult<String>> {
        Box::pin(async move {
            assert!(self.scope.lock().unwrap().as_ref().unwrap().same_generation(scope));
            assert_eq!(session.active_user, self.identity.user);
            assert_eq!(session.cookie_header, "session=fixture-secret");
            assert_eq!(target.project, self.identity.project);
            assert_eq!(target.path, "/ai/v3/responses/");
            assert_eq!(page, format!("https://framer.com/projects/{}", target.project.0));
            assert_eq!(cookies, CURRENT_COOKIES);
            assert!(!cancel.is_cancelled());
            assert_eq!(self.bearers.fetch_add(1, Ordering::SeqCst), 0);
            PreparationResult::finished(Ok("fixture-bearer".into()))
        })
    }
    fn authorize<'a>(&'a self, _: &'a AccountScope, _: &'a Session,
        _: &'a CredentialTarget, _: &'a PreparationControl, _: BrowserAuthorization)
        -> FutureResult<'a, PreparationResult<PreparedCredentials>> {
        self.authorizations.fetch_add(1, Ordering::SeqCst);
        panic!("a refused baseline must not issue native credentials")
    }
}

impl pair::control::ControlFactory for RefusalFactory {
    fn build(&self, scope: AccountScope, proxy: Option<&str>)
        -> Result<Arc<dyn pair::control::Control>, Failure> {
        assert_eq!(proxy, Some(FIXTURE_PROXY));
        assert_eq!(scope.account(), self.0.identity.account);
        assert_eq!(scope.active_user(), &self.0.identity.user);
        assert!(scope.matches_egress(&NativeEgress::Proxy(FIXTURE_PROXY)));
        assert_eq!(self.0.builds.fetch_add(1, Ordering::SeqCst), 0);
        assert!(self.0.scope.lock().unwrap().replace(scope).is_none());
        Ok(self.0.clone())
    }
}

struct RefusalWindow {
    inner: Window,
    baseline: Mutex<Option<framer_host::sidecar::observer::comparison::BaselineReport>>,
}

async fn refusal_case(drop_waiter: bool) {
    let pipe = RefusalPipe::new();
    let id = identity();
    let (store, _, _) = fixture(&id, None);
    let control = Arc::new(RefusalControl {
        identity: id.clone(), scope: Mutex::new(None), builds: AtomicUsize::new(0),
        bearers: AtomicUsize::new(0), authorizations: AtomicUsize::new(0),
    });
    let window = Arc::new(RefusalWindow {
        inner: Window {
            selected: Mutex::new(Some(Selection {
                identity: id.clone(), context: context(), proxy: Some(FIXTURE_PROXY.into()),
            })),
            admissions: AtomicUsize::new(0), reports: Mutex::new(Vec::new()),
            recorded: Semaphore::new(0),
        },
        baseline: Mutex::new(None),
    });
    let slots = Arc::new(Semaphore::new(1));
    let outcome = std::panic::AssertUnwindSafe(async {
        let mut waiter = Some(pair::launch_with_control(spec(id), "baseline-turn".into(),
            pair::Dependencies { authority: window.clone(), store: store.clone(),
                observer: pipe.observer.as_ref().unwrap().clone(),
                permit: slots.clone().try_acquire_owned().unwrap() },
            Arc::new(RefusalFactory(control.clone()))).unwrap());
        pipe.event("baseline_entered").await;
        if drop_waiter { drop(waiter.take()); }
        assert!(window.inner.reports.lock().unwrap().is_empty());
        assert_eq!(slots.available_permits(), 0);
        pipe.release("release_baseline");
        pipe.event("close_entered").await;
        let scope = control.scope.lock().unwrap().as_ref().unwrap().clone();
        tokio::time::timeout(BOUND, scope.closed()).await.unwrap().unwrap();
        assert!(window.inner.reports.lock().unwrap().is_empty(),
            "refusal was published before the observed browser close joined");
        assert!(window.baseline.lock().unwrap().is_none());
        assert_eq!(control.authorizations.load(Ordering::SeqCst), 0);
        pipe.release("release_close");
        signalled(&window.inner.recorded).await;
        if let Some(waiter) = waiter {
            let report = tokio::time::timeout(BOUND, waiter).await.unwrap();
            assert_eq!(report.failure, Some(Failure::Dispatch));
            assert!(report.account_closed && report.browser_cleanup_confirmed);
            assert!(report.native.is_none() && !report.native_nonempty_text);
            assert_refusal(report.baseline.as_ref().unwrap());
        }
        assert_refusal(window.baseline.lock().unwrap().as_ref().unwrap());
        assert_eq!(*window.inner.reports.lock().unwrap(), vec![Outcome {
            failure: Some(Failure::Dispatch), baseline: true, native: false,
            browser_cleanup: true, account_closed: true,
        }]);
        assert_eq!(window.inner.admissions.load(Ordering::SeqCst), 1);
        assert_eq!(store.loads.load(Ordering::SeqCst), 1);
        assert_eq!(control.builds.load(Ordering::SeqCst), 1);
        assert_eq!(control.bearers.load(Ordering::SeqCst), 1);
        assert_eq!(control.authorizations.load(Ordering::SeqCst), 0);
        assert_eq!(slots.available_permits(), 1);
        pipe.event("browser_closed").await;
        pipe.event("scope_forgotten").await;
        assert_eq!(std::fs::read_to_string(pipe.trace.join("commands")).unwrap(),
            "prepare_inference_authorization:baseline_context\nbrowser_baseline_once:None\n");
    }).catch_unwind().await;
    pipe.finish().await;
    if let Err(panic) = outcome { std::panic::resume_unwind(panic); }
}

#[tokio::test]
async fn baseline_security_refusal_never_issues_native_credentials() {
    refusal_case(false).await;
}

#[tokio::test]
async fn dropped_baseline_waiter_retains_close_and_refusal_publication() {
    refusal_case(true).await;
}
impl pair::ComparisonWindow for RefusalWindow {
    fn admit(&self) -> FutureResult<'_, Result<Selection, Failure>> {
        pair::ComparisonWindow::admit(&self.inner)
    }
    fn record(&self, report: &pair::Report) -> FutureResult<'_, Result<(), Failure>> {
        *self.baseline.lock().unwrap() = report.baseline.clone();
        pair::ComparisonWindow::record(&self.inner, report)
    }
}

// Reuse the real observer protocol/writer and its existing fake browser.
// Only synthetic host commands are implemented; there is no SDK or HTTP.
const REFUSAL_SIDECAR: &str = r#"
import asyncio, os, select, signal, sys
from pathlib import Path
sys.dont_write_bytecode = True
import observed_sidecar as obs
import contextual_authorization as contextual
from observer_pipe_writer import AsyncReplyWriter
from observed_sidecar_tests import FakeBrowser, FakeHost
signal.alarm(15)
ROOT = Path(os.environ['FRAMERIZE_STATE'])
TRACE = ROOT.parent
def note(name):
    (TRACE / name).write_text('fixture\n')
async def released(name):
    while not (TRACE / name).exists():
        await asyncio.sleep(0.005)

class Browser(FakeBrowser):
    async def send(self, *args):
        note('close_entered')
        await released('release_close')
        await super().send(*args)
        note('browser_closed')

class Host(FakeHost):
    async def handle(self, req):
        scope = self.owner.scope_for(req)
        assert req.get('proxy') == 'http://127.0.0.1:9'
        if req['cmd'] == 'prepare_inference_authorization':
            assert req['purpose'] == 'baseline_context', 'unexpected native token command'
            assert req['wrapper_url'] == contextual.COMPARISON_WRAPPER
            assert req['wrapper_export'] == 'b'
            assert req['cookie_header'] == 'session=fixture-secret'
            first, second = dict(req['body']), dict(req['native_body'])
            assert first.pop('turnId') == 'baseline-turn'
            assert second.pop('turnId') == 'fixture-turn'
            assert first == second and first['request']['stream'] is False
            self.body, self.identity = req['body'], scope.identity()
            self.binding = dict(source='framer_browser_comparison_v1',
                context_handle='fixture-context', project=req['project'], path=req['path'])
            scope.begin_launch()
            scope.browser_started(Browser())
            self.accounts[scope.account] = 'synthetic account only'
            scope.comparison_next = 'context_ready'
            return dict(self.binding, purpose='baseline_context',
                page_url='https://framer.com/projects/' + req['project'],
                cookie_header='session=fixture-secret; __cuid=fixture-device')
        assert req['cmd'] == 'browser_baseline_once'
        assert scope.identity() == self.identity and req['body'] == self.body
        assert req['bearer'] == 'fixture-bearer'
        assert req['cookie_header'] == 'session=fixture-secret; __cuid=fixture-device'
        for key in ('context_handle', 'project', 'path'):
            assert req[key] == self.binding[key]
        note('baseline_entered')
        await released('release_baseline')
        return dict(self.binding, accepted=False, status=403, content_type='json',
            response_complete=True, complete_json=False, nonempty_text=False,
            body_bytes=231, transport_starts=1, transport_finishes=1, fetch_settled=True,
            refusal={'policy': 'castle', 'basis': 'framer_security_envelope'}, failure='upstream',
            body='fixture-private-response', token='fixture-private-token')

class Owner(obs.ObservedHost):
    def accept(self, req):
        if req['cmd'] in ('prepare_inference_authorization', 'browser_baseline_once'):
            with (TRACE / 'commands').open('a') as out:
                out.write(req['cmd'] + ':' + str(req.get('purpose')) + '\n')
        super().accept(req)
    async def forget(self, request_id, scope):
        await super().forget(request_id, scope)
        note('scope_forgotten')

class Input:
    def __init__(self):
        self.pending = b''
    def readline(self):
        while b'\n' not in self.pending:
            if (TRACE / 'shutdown').exists():
                return ''
            if not select.select([sys.stdin], [], [], 0.01)[0]:
                continue
            part = os.read(sys.stdin.fileno(), 65536)
            if not part:
                return ''
            self.pending += part
            assert len(self.pending) <= 131072, 'fixture command exceeds bound'
        line, self.pending = self.pending.split(b'\n', 1)
        return line.decode() + '\n'

async def main():
    assert os.environ['FRAMERIZE_SIDECAR_MODE'] == obs.MODE
    obs.verify_close_protocol = lambda: None  # Explicit fake browser protocol.
    host = Host()
    owner = Owner(host, ROOT, os.environ['FRAMERIZE_SIDECAR_INCARNATION'],
                  os.environ['FRAMERIZE_OBSERVED_PROFILE_NONCE'])
    host.owner = owner
    (TRACE / 'pid').write_text(str(os.getpid()))
    await owner.loop(AsyncReplyWriter(sys.stdout), Input())

asyncio.run(main())
note('exited')
"#;

struct RefusalPipe {
    _temp: Temp,
    trace: std::path::PathBuf,
    observer: Option<ObserverFactory>,
    bridge: Option<Arc<SidecarBridge>>,
}

impl RefusalPipe {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("comparison-refusal-{}", uuid::Uuid::new_v4()));
        let mut directory = std::fs::DirBuilder::new();
        #[cfg(unix)] {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        directory.create(&root).unwrap();
        let temp = Temp(root.clone());
        Self::prepare(temp)
    }

    fn prepare(temp: Temp) -> Self {
        for (name, text) in [
            ("observed_sidecar.py", include_str!("../../../scripts/observed_sidecar.py")),
            ("contextual_authorization.py", include_str!("../../../scripts/contextual_authorization.py")),
            ("observer_pipe_writer.py", include_str!("../../../scripts/observer_pipe_writer.py")),
            ("observed_sidecar_tests.py", include_str!("../../../reference/fixtures/observed_sidecar_tests.py")),
            ("fixture.py", REFUSAL_SIDECAR),
        ] {
            std::fs::write(temp.0.join(name), text).unwrap();
        }
        let bridge = Arc::new(SidecarBridge::new_observed_one_shot(
            &temp.0, temp.0.join("fixture.py")).unwrap());
        let trace = std::fs::read_dir(&temp.0).unwrap().map(|entry| entry.unwrap().path())
            .find(|path| path.is_dir()).unwrap();
        let observer = bridge.observer_factory(ObserverLimits {
            scopes: 1, commands: 2, cleanup_timeout: BOUND,
        }).unwrap();
        Self { _temp: temp, trace, observer: Some(observer), bridge: Some(bridge) }
    }

    fn release(&self, name: &str) {
        std::fs::write(self.trace.join(name), b"fixture\n").unwrap();
    }

    async fn event(&self, name: &str) {
        tokio::time::timeout(BOUND, async {
            while !self.trace.join(name).is_file() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.unwrap_or_else(|_| panic!("missing fake observer event: {name}"));
    }

    async fn finish(mut self) {
        self.release("release_baseline");
        self.release("release_close");
        self.release("shutdown");
        drop(self.observer.take());
        drop(self.bridge.take());
        let Ok(value) = std::fs::read_to_string(self.trace.join("pid")) else { return; };
        let pid: libc::pid_t = value.parse().unwrap();
        assert!(pid > 1);
        self.event("exited").await;
        // The bridge owns/reaps this fixture child. Observe absence only;
        // do not signal a PID or treat forced cleanup as a passing receipt.
        tokio::time::timeout(BOUND, async {
            loop {
                if unsafe { libc::kill(pid, 0) } == -1 {
                    assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("fake observer child was not reaped");
    }
}
