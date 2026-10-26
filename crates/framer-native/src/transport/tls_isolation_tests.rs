use super::*;
use serde_json::json;
use std::io::{BufRead, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;

const PATH: &str = "/ai/v3/openai/responses/";
const WAIT: Duration = Duration::from_secs(5);

struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct TlsServer {
    process: Process,
    origin: String,
    certificate: reqwest::Certificate,
}

impl TlsServer {
    fn start(version: &str) -> Self {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/experiments/native_tls_isolation/server.py");
        let mut process = Process(Command::new("python3").args(["-I", "-B"])
            .arg(script).args(["--tls-version", version])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit())
            .spawn().expect("start local TLS fixture"));
        let output = process.0.stdout.take().unwrap();
        let (send, receive) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut line = String::new();
            let result = std::io::BufReader::new(output).take(16385)
                .read_line(&mut line).map(|_| line);
            let _ = send.send(result);
        });
        let ready = receive.recv_timeout(Duration::from_secs(10));
        if ready.is_err() {
            let _ = process.0.kill();
            let _ = process.0.wait();
        }
        reader.join().expect("join TLS readiness reader");
        let line = ready.expect("TLS server readiness deadline").unwrap();
        assert!(line.len() <= 16384, "TLS readiness exceeds limit");
        let ready: Value = serde_json::from_str(&line).expect("TLS readiness JSON");
        let port = u16::try_from(ready["port"].as_u64().unwrap()).unwrap();
        assert_ne!(port, 0);
        let certificate = reqwest::Certificate::from_pem(
            ready["certificate_pem"].as_str().unwrap().as_bytes()).unwrap();
        Self { process, origin: format!("https://127.0.0.1:{port}"), certificate }
    }

    fn finish(mut self) {
        self.process.0.stdin.take().unwrap().write_all(b"stop\n").unwrap();
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(status) = self.process.0.try_wait().unwrap() {
                assert!(status.success(), "TLS fixture did not exit successfully");
                return;
            }
            assert!(Instant::now() < deadline, "TLS fixture cleanup deadline");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn client(&self, account: AccountId, user: &str) -> NativeHttp {
        NativeHttp::build_with_builder(account, FramerUserId(user.into()),
            self.origin.clone(), NativeHttpLimits {
                request_timeout: WAIT, connect_timeout: WAIT, ..Default::default()
            }, true, NativeEgress::Direct,
            reqwest::Client::builder().add_root_certificate(self.certificate.clone()))
            .expect("account TLS client")
    }
}

fn run(work: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(work);
}

fn session(user: &str) -> Session {
    Session { active_user: FramerUserId(user.into()), cookie_header: format!("session={user}") }
}

async fn send(client: &NativeHttp, account: AccountId, user: &str, close: bool) -> Value {
    let credentials = session(user);
    let bearer = format!("fixture-bearer-{user}");
    let token = format!("fixture-token-{}", uuid::Uuid::new_v4());
    let body = json!({"fixture_close": close, "fixture_account": user});
    let (status, _, text) = client.ai_post(&account, PATH,
        AccountRequestAuth::new(account, &credentials, &bearer), &body,
        FreshCastleToken::from_browser(account, CastleToken(token.clone())),
        NativeCancellation::new()).await.expect("local TLS request");
    assert_eq!(status, 200);
    let reply: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reply["authorization"], format!("Bearer {bearer}"));
    assert_eq!(reply["cookie"], "", "default inference must not transmit session cookies");
    assert_eq!(reply["request_body"]["fixture_account"], user);
    assert_eq!(reply["request_body"]["castleToken"], token);
    tokio::task::yield_now().await;
    reply
}

#[test]
fn account_tls_clients_reuse_only_their_own_connections() {
    for version in ["1.2", "1.3"] {
        let server = TlsServer::start(version);
        run(async {
            let a = AccountId(uuid::Uuid::new_v4());
            let b = AccountId(uuid::Uuid::new_v4());
            let first = server.client(a, "alpha");
            let second = server.client(b, "beta");
            let a1 = send(&first, a, "alpha", false).await;
            let b1 = send(&second, b, "beta", false).await;
            let a2 = send(&first, a, "alpha", false).await;
            let b2 = send(&second, b, "beta", false).await;
            assert_ne!(a1["connection"], b1["connection"], "account pools merged");
            assert_eq!(a1["connection"], a2["connection"], "first account did not reuse");
            assert_eq!(b1["connection"], b2["connection"], "second account did not reuse");
            assert_eq!(a2["requests_on_connection"], 2);
            assert_eq!(b2["requests_on_connection"], 2);
            assert_eq!(a1["tls_version"], format!("TLSv{version}"));
            assert_eq!(b1["tls_version"], format!("TLSv{version}"));
        });
        server.finish();
    }
}

#[test]
fn forced_tls_reconnections_preserve_account_ownership() {
    for version in ["1.2", "1.3"] {
        let server = TlsServer::start(version);
        run(async {
            let a = AccountId(uuid::Uuid::new_v4());
            let b = AccountId(uuid::Uuid::new_v4());
            let first = server.client(a, "alpha");
            let second = server.client(b, "beta");
            let a1 = send(&first, a, "alpha", true).await;
            let a2 = send(&first, a, "alpha", true).await;
            let b1 = send(&second, b, "beta", true).await;
            assert_ne!(a1["connection"], a2["connection"]);
            assert_ne!(a2["connection"], b1["connection"]);
            assert_eq!(a1["resumed"], false);
            assert_eq!(b1["resumed"], false, "new account resumed another client's session");
            println!("tls_resumption_observation {}", json!({"protocol": version,
                "same_client_resumed": a2["resumed"], "new_account_resumed": b1["resumed"]}));
        });
        server.finish();
    }
}

#[test]
fn tls_certificate_verification_remains_required() {
    let server = TlsServer::start("1.3");
    run(async {
        let account = AccountId(uuid::Uuid::new_v4());
        let credentials = session("alpha");
        let untrusted = NativeHttp::build(account, credentials.active_user.clone(),
            server.origin.clone(), NativeHttpLimits {
                request_timeout: WAIT, connect_timeout: WAIT, ..Default::default()
            }, true, NativeEgress::Direct).unwrap();
        let result = untrusted.ai_post(&account, PATH,
            AccountRequestAuth::new(account, &credentials, "fixture-bearer"),
            &json!({"fixture_close": true}),
            FreshCastleToken::from_browser(account, CastleToken("fixture-token".into())),
            NativeCancellation::new()).await;
        assert_eq!(result.unwrap_err(), "native-http: transport failed");
        let trusted = server.client(account, "alpha");
        let reply = send(&trusted, account, "alpha", true).await;
        assert_eq!(reply["requests_on_connection"], 1);
    });
    server.finish();
}
