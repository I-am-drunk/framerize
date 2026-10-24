use super::*;
#[path = "control_authorization_fixture.rs"]
mod fixture;
use fixture::*;
use crate::experimental_native::native_http::{AccountRequestAuth, NativeCancellation,
    NativeHttp, NativeHttpLimits};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const PAGE: &str = "https://framer.com/login";
const DEVICE: &str = "Raw-Device_01";
const JAR: &str = "session=fixture-only-cookie; __cuid=Raw-Device_01; current=fresh";

fn target() -> CredentialTarget {
    CredentialTarget { project: ProjectId("fixture-project".into()),
        path: "/ai/v3/openai/responses/" }
}

fn identity(scope: &AccountScope) -> ResidentLeaseIdentity {
    ResidentLeaseIdentity::new(scope.account(), 1,
        "fixture-host-generation".into(), 1, Uuid::from_u128(2)).unwrap()
}

fn context(scope: &AccountScope, started: Instant) -> TrustedResidentContext {
    TrustedResidentContext::new(scope, identity(scope), target(), PAGE.into(), DEVICE.into(),
        NativeEgress::Direct, started).unwrap()
}

fn observation<'a>(identity: &'a ResidentLeaseIdentity,
    target: &'a CredentialTarget) -> ResidentObservation<'a> {
    ResidentObservation::from_unverified_reply(identity, &target.project,
        target.path, PAGE, DEVICE, JAR, NativeEgress::Direct)
}

fn apply_observation_drift(case: usize, actual: &mut ResidentObservation<'_>) {
    match case {
        7 => actual.page_url = "https://framer.com/login/",
        8 => actual.page_url = "https://framer.com/projects/fixture-project",
        9 => actual.page_url = "https://framer.com/login?changed=1",
        10 => actual.device_id = "raw-device_01",
        11 => actual.cookie_header = "session=other; __cuid=Raw-Device_01",
        12 => actual.cookie_header = "__cuid=Raw-Device_01",
        13 => actual.cookie_header = "session=fixture-only-cookie",
        14 => actual.cookie_header = "session=fixture-only-cookie; __cuid=other",
        15 => actual.cookie_header = "session=fixture-only-cookie; __cuid=Raw-Device_01; __cuid=Raw-Device_01",
        16 => actual.cookie_header = "session=fixture-only-cookie; session=fixture-only-cookie; __cuid=Raw-Device_01",
        17 => actual.egress = NativeEgress::Proxy("http://fixture.invalid:8080"),
        18 => actual.cookie_header = "session=fixture-only-cookie; __cuid=Raw-Device_01 ",
        19 => actual.cookie_header = "session=fixture-only-cookie; __cuid=Raw%2DDevice_01",
        20 => actual.cookie_header = "session=fixture-only-cookie; __cuid=Raw-Device_01\r\n",
        21 => actual.page_url = "https://FRAMER.com/login",
        _ => {}
    }
}

fn grant(api: &BoundControlClient, scope: &AccountScope, started: Instant) -> ResidentGrant {
    ResidentGrant::new_checked(api, context(scope, started), &session(),
        observation(&identity(scope), &target())).unwrap()
}

#[tokio::test]
async fn exact_login_binding_accepts_without_relaxing_editor_guard() {
    let (client, scope) = scope(NativeEgress::Direct);
    let api = BoundControlClient::new(scope.clone(), NativeEgress::Direct).unwrap();
    let checked = grant(&api, &scope, Instant::now());
    assert!(checked.current_header == JAR);
    let editor = CredentialTarget { project: target().project, path: "/ai/v3/responses/" };
    assert!(!editor.matches_page(PAGE));
    assert!(editor.matches_page("https://framer.com/projects/fixture-project"));
    close(&client, &scope).await;
}

#[tokio::test]
async fn grant_rejects_identity_page_device_target_jar_and_route_drift() {
    let (client, scope) = scope(NativeEgress::Direct);
    let api = BoundControlClient::new(scope.clone(), NativeEgress::Direct).unwrap();
    for case in 0..22 {
        let mut actual_identity = identity(&scope);
        let mut actual_target = target();
        match case {
            0 => actual_identity.account = AccountId(Uuid::new_v4()),
            1 => actual_identity.application_generation += 1,
            2 => actual_identity.host_generation = "other-host".into(),
            3 => actual_identity.account_incarnation += 1,
            4 => actual_identity.lease = Uuid::new_v4(),
            5 => actual_target.project = ProjectId("other-project".into()),
            6 => actual_target.path = "/ai/v3/messages/",
            _ => {}
        }
        let mut actual = observation(&actual_identity, &actual_target);
        apply_observation_drift(case, &mut actual);
        let checked = ResidentGrant::new_checked(&api, context(&scope, Instant::now()),
            &session(), actual);
        assert!(matches!(checked, Err(PreparationFailure::Rejected)), "case {case}");
    }
    close(&client, &scope).await;
}

#[tokio::test]
async fn same_account_on_another_native_owner_and_wrong_session_are_rejected() {
    let (client, scope) = scope(NativeEgress::Direct);
    let other_client = crate::experimental_native::native_account::NativeAccountClient::new().unwrap();
    let other = other_client.open_account(scope.account(), session(), NativeHttpLimits::default(),
        NativeEgress::Direct).unwrap();
    let api = BoundControlClient::new(other.clone(), NativeEgress::Direct).unwrap();
    let actual_id = identity(&scope);
    let target = target();
    let result = ResidentGrant::new_checked(&api, context(&scope, Instant::now()),
        &session(), observation(&actual_id, &target));
    assert!(matches!(result, Err(PreparationFailure::Rejected)));
    let api = BoundControlClient::new(scope.clone(), NativeEgress::Direct).unwrap();
    let mut wrong_session = session();
    wrong_session.active_user.0 = "other-user".into();
    let result = ResidentGrant::new_checked(&api, context(&scope, Instant::now()),
        &wrong_session, observation(&actual_id, &target));
    assert!(matches!(result, Err(PreparationFailure::Rejected)));
    close(&other_client, &other).await;
    close(&client, &scope).await;
}

#[tokio::test]
async fn supplied_egress_must_equal_the_captured_proxy_bytes() {
    let proxy = "http://fixture.invalid:8080";
    let (client, scope) = scope(NativeEgress::Proxy(proxy));
    let api = BoundControlClient::new(scope.clone(), NativeEgress::Proxy(proxy)).unwrap();
    let actual_id = identity(&scope);
    let target = target();
    for candidate in [proxy, "http://fixture.invalid:8080/", "http://other.invalid:8080"] {
        let expected = TrustedResidentContext::new(&scope, identity(&scope), self::target(),
            PAGE.into(), DEVICE.into(), NativeEgress::Proxy(proxy), Instant::now()).unwrap();
        let mut actual = observation(&actual_id, &target);
        actual.egress = NativeEgress::Proxy(candidate);
        let checked = ResidentGrant::new_checked(&api, expected, &session(), actual);
        assert_eq!(checked.is_ok(), candidate == proxy);
    }
    assert!(TrustedResidentContext::new(&scope, identity(&scope), self::target(),
        PAGE.into(), DEVICE.into(), NativeEgress::Direct, Instant::now()).is_err());
    close(&client, &scope).await;
}

async fn serve_retaining_listener(server: tokio::net::TcpListener, delay: Duration)
    -> (tokio::net::TcpListener, String) {
    tokio::time::timeout(BOUND, async move {
        let (mut socket, _) = server.accept().await.unwrap();
        let request = read_request(&mut socket).await;
        tokio::time::sleep(delay).await;
        socket.write_all(response(200, &token().to_string(), "").as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
        (server, request)
    }).await.unwrap()
}

#[tokio::test]
async fn checked_jar_acquires_one_bearer_retained_across_both_arms() {
    let (client, scope) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = bound(&scope, server.local_addr().unwrap(), NativeEgress::Direct);
    let checked = grant(&api, &scope, Instant::now());
    let expected = scope.clone();
    let worker = tokio::spawn(serve_retaining_listener(server, Duration::ZERO));
    let pending = client.prepare_owned(&scope, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            let retained = api.resident_bearer(checked, &control).await.value.unwrap();
            let baseline_bearer = retained.as_str().to_owned();
            let prepared = api.authorize_resident_token(retained, &session,
                observation(&identity(&expected), &target()),
                CastleToken("fixture-token".into()), &control).unwrap();
            assert!(prepared.material.bearer == baseline_bearer);
            assert!(baseline_bearer == "fixture-bearer");
            let current = prepared.current_cookies.unwrap().into_session(&session).unwrap();
            assert!(current.cookie_header == JAR);
            PreparationResult::finished(Ok(()))
        }).unwrap();
    tokio::time::timeout(BOUND, pending).await.unwrap().unwrap();
    let (server, request) = tokio::time::timeout(BOUND, worker).await.unwrap().unwrap();
    assert!(request.lines().any(|line| line.split_once(':').is_some_and(|(name, value)|
        name.eq_ignore_ascii_case("cookie") && value.trim_start() == JAR)));
    assert!(server.accept().now_or_never().is_none());
    close(&client, &scope).await;
}

fn retained_fixture(api: &BoundControlClient, scope: &AccountScope) -> ResidentBearer {
    ResidentBearer { grant: grant(api, scope, Instant::now()), value: "fixture-bearer".into() }
}

async fn separate_mint_age(delay: Duration, max_age: Duration, accepted: bool) {
    let (client, scope) = scope(NativeEgress::Direct);
    let api = BoundControlClient::new(scope.clone(), NativeEgress::Direct).unwrap();
    let expected = scope.clone();
    let pending = client.prepare_owned(&scope, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            let retained = ResidentBearer { grant: grant(&api, &expected,
                Instant::now() - Duration::from_secs(60)), value: "fixture-bearer".into() };
            let mint = api.begin_resident_mint(retained, &session,
                observation(&identity(&expected), &target()), &control).unwrap();
            tokio::time::sleep(delay).await;
            PreparationResult::finished(api.authorize_resident_mint(mint, &session,
                observation(&identity(&expected), &target()),
                CastleToken("fixture-token".into()), &control))
        }).unwrap();
    let prepared = tokio::time::timeout(BOUND, pending).await.unwrap().unwrap();
    let sink = listener().await;
    let native = NativeHttp::for_loopback(scope.account(), session().active_user,
        sink.local_addr().unwrap(), NativeHttpLimits {
            token_max_age: max_age, ..NativeHttpLimits::default()
        }).unwrap();
    let mut sink = Some(sink);
    let worker = if accepted { Some(tokio::spawn(serve_retaining_listener(sink.take().unwrap(), Duration::ZERO))) }
        else { None };
    let current = prepared.current_cookies.unwrap().into_session(&session()).unwrap();
    let result = tokio::time::timeout(BOUND, native.ai_post(&scope.account(), target().path,
        AccountRequestAuth::new(scope.account(), &current, &prepared.material.bearer),
        &serde_json::json!({"stream": false}), prepared.material.castle,
        NativeCancellation::new())).await.unwrap();
    assert_eq!(result.is_ok(), accepted);
    if let Some(worker) = worker {
        let (sink, _) = tokio::time::timeout(BOUND, worker).await.unwrap().unwrap();
        assert!(sink.accept().now_or_never().is_none());
    } else {
        assert_eq!(result.unwrap_err(), "native-http: fresh Castle token required");
        assert!(sink.unwrap().accept().now_or_never().is_none());
    }
    close(&client, &scope).await;
}

#[tokio::test]
async fn native_mint_starts_after_old_bearer_and_baseline_without_reauth() {
    separate_mint_age(Duration::ZERO, Duration::from_secs(10), true).await;
}

#[tokio::test]
async fn separate_mint_roundtrip_is_not_restamped_on_authorization() {
    separate_mint_age(Duration::from_millis(60), Duration::from_millis(20), false).await;
}

#[tokio::test]
async fn separate_mint_rejects_changed_jar_before_and_after_rpc() {
    let (client, scope) = scope(NativeEgress::Direct);
    let api = BoundControlClient::new(scope.clone(), NativeEgress::Direct).unwrap();
    let expected = scope.clone();
    let pending = client.prepare_owned(&scope, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            let id = identity(&expected);
            let target = target();
            let mut drift = observation(&id, &target);
            drift.cookie_header = "session=fixture-only-cookie; __cuid=Raw-Device_01; current=changed";
            assert!(api.begin_resident_mint(retained_fixture(&api, &expected),
                &session, drift, &control).is_err());
            let mint = api.begin_resident_mint(retained_fixture(&api, &expected),
                &session, observation(&id, &target), &control).unwrap();
            let mut drift = observation(&id, &target);
            drift.cookie_header = "session=fixture-only-cookie; __cuid=Raw-Device_01; current=changed";
            assert!(api.authorize_resident_mint(mint, &session, drift,
                CastleToken("fixture-token".into()), &control).is_err());
            PreparationResult::finished(Ok(()))
        }).unwrap();
    tokio::time::timeout(BOUND, pending).await.unwrap().unwrap();
    close(&client, &scope).await;
}

#[test]
fn browser_timestamp_constructor_rejects_future_birth_time() {
    let future = Instant::now() + Duration::from_secs(60);
    assert!(FreshCastleToken::from_browser_at(AccountId(Uuid::new_v4()),
        CastleToken("fixture-token".into()), future).is_err());
}

#[tokio::test]
async fn authorization_rechecks_binding_session_and_exact_bearer_jar() {
    let (client, scope) = scope(NativeEgress::Direct);
    let api = BoundControlClient::new(scope.clone(), NativeEgress::Direct).unwrap();
    let expected = scope.clone();
    let pending = client.prepare_owned(&scope, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            let actual_id = identity(&expected);
            let target = target();
            for case in 7..25 {
                let retained = retained_fixture(&api, &expected);
                let mut actual = observation(&actual_id, &target);
                let mut current_session = session.clone();
                apply_observation_drift(case, &mut actual);
                match case {
                    22 => actual.cookie_header = "session=fixture-only-cookie; __cuid=Raw-Device_01; current=changed",
                    23 => current_session.cookie_header.push_str("; other=changed"),
                    24 => current_session.active_user.0 = "other-user".into(),
                    _ => {}
                }
                let result = api.authorize_resident_token(retained, &current_session, actual,
                    CastleToken("fixture-token".into()), &control);
                assert!(matches!(result, Err(PreparationFailure::Rejected)), "case {case}");
            }
            PreparationResult::finished(Ok(()))
        }).unwrap();
    tokio::time::timeout(BOUND, pending).await.unwrap().unwrap();
    close(&client, &scope).await;
}

#[tokio::test]
async fn identity_and_owner_context_reject_invalid_inputs() {
    let (client, scope) = scope(NativeEgress::Direct);
    assert!(ResidentLeaseIdentity::new(scope.account(), 0,
        "fixture-host".into(), 1, Uuid::from_u128(2)).is_err());
    assert!(ResidentLeaseIdentity::new(scope.account(), 1,
        "fixture-host".into(), 0, Uuid::from_u128(2)).is_err());
    assert!(ResidentLeaseIdentity::new(scope.account(), 1,
        "fixture-host".into(), 1, Uuid::nil()).is_err());
    assert!(ResidentLeaseIdentity::new(scope.account(), 1,
        "fixture\nhost".into(), 1, Uuid::from_u128(2)).is_err());
    for case in 0..7 {
        let mut identity = identity(&scope);
        let mut target = target();
        let mut page = PAGE;
        let mut device = DEVICE;
        let mut started = Instant::now();
        match case {
            0 => identity.account = AccountId(Uuid::new_v4()),
            1 => target.path = "/auth/ai/access-token",
            2 => target.project.0 = "../other".into(),
            3 => page = "http://framer.com/login",
            4 => page = "https://other.invalid/login",
            5 => device = " Raw-Device_01",
            _ => started += Duration::from_secs(60),
        }
        let result = TrustedResidentContext::new(&scope, identity, target,
            page.into(), device.into(), NativeEgress::Direct, started);
        assert!(matches!(result, Err(PreparationFailure::Rejected)), "case {case}");
    }
    close(&client, &scope).await;
}

async fn age_reaches_native_guard(ipc_age: Duration, bearer_wait: Duration, max_age: Duration) {
    let (client, scope) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = bound(&scope, server.local_addr().unwrap(), NativeEgress::Direct);
    let checked = grant(&api, &scope, Instant::now() - ipc_age);
    let expected = scope.clone();
    let worker = tokio::spawn(serve_retaining_listener(server, bearer_wait));
    let pending = client.prepare_owned(&scope, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            let retained = api.resident_bearer(checked, &control).await.value.unwrap();
            PreparationResult::finished(api.authorize_resident_token(retained, &session,
                observation(&identity(&expected), &target()),
                CastleToken("fixture-token".into()), &control))
        }).unwrap();
    let prepared = tokio::time::timeout(BOUND, pending).await.unwrap().unwrap();
    let (auth_server, _) = tokio::time::timeout(BOUND, worker).await.unwrap().unwrap();
    assert!(auth_server.accept().now_or_never().is_none());
    let sink = listener().await;
    let native = NativeHttp::for_loopback(scope.account(), session().active_user,
        sink.local_addr().unwrap(), NativeHttpLimits {
            token_max_age: max_age, ..NativeHttpLimits::default()
        }).unwrap();
    let current = prepared.current_cookies.unwrap().into_session(&session()).unwrap();
    let result = tokio::time::timeout(BOUND, native.ai_post(&scope.account(), target().path,
        AccountRequestAuth::new(scope.account(), &current, &prepared.material.bearer),
        &serde_json::json!({"stream": false}), prepared.material.castle,
        NativeCancellation::new())).await.unwrap();
    assert_eq!(result.unwrap_err(), "native-http: fresh Castle token required");
    assert!(sink.accept().now_or_never().is_none());
    close(&client, &scope).await;
}

#[tokio::test]
async fn pre_acquire_age_survives_ipc_and_token_conversion() {
    age_reaches_native_guard(Duration::from_secs(10), Duration::ZERO,
        Duration::from_secs(1)).await;
}

#[tokio::test]
async fn retained_bearer_wait_counts_toward_castle_age() {
    age_reaches_native_guard(Duration::ZERO, Duration::from_millis(60),
        Duration::from_millis(20)).await;
}

#[tokio::test]
async fn token_and_bearer_limits_reject_empty_or_oversized_material() {
    let (client, scope) = scope(NativeEgress::Direct);
    let api = BoundControlClient::new(scope.clone(), NativeEgress::Direct).unwrap();
    let expected = scope.clone();
    let pending = client.prepare_owned(&scope, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            for case in 0..5 {
                let mut retained = retained_fixture(&api, &expected);
                let mut token = CastleToken("fixture-token".into());
                match case {
                    0 => token.0.clear(),
                    1 => token.0 = "x".repeat(CREDENTIAL_LIMIT + 1),
                    2 => retained.value.clear(),
                    3 => retained.value = "x".repeat(CREDENTIAL_LIMIT + 1),
                    _ => {
                        token.0 = "x".repeat(CREDENTIAL_LIMIT);
                        retained.value = "x".repeat(CREDENTIAL_LIMIT);
                    }
                }
                let result = api.authorize_resident_token(retained, &session,
                    observation(&identity(&expected), &target()), token, &control);
                assert_eq!(result.is_ok(), case == 4);
            }
            PreparationResult::finished(Ok(()))
        }).unwrap();
    tokio::time::timeout(BOUND, pending).await.unwrap().unwrap();
    close(&client, &scope).await;
}

#[tokio::test]
async fn current_and_seeded_jars_enforce_the_16kib_limit() {
    let (client, scope) = scope(NativeEgress::Direct);
    let api = BoundControlClient::new(scope.clone(), NativeEgress::Direct).unwrap();
    let actual_id = identity(&scope);
    let target = target();
    for size in [CREDENTIAL_LIMIT, CREDENTIAL_LIMIT + 1] {
        let jar = format!("{JAR}; pad={}", "x".repeat(size - JAR.len() - 6));
        let mut actual = observation(&actual_id, &target);
        actual.cookie_header = &jar;
        let checked = ResidentGrant::new_checked(&api, context(&scope, Instant::now()),
            &session(), actual);
        assert_eq!(checked.is_ok(), size == CREDENTIAL_LIMIT);
    }
    let mut oversized = session();
    oversized.cookie_header.push_str(&format!("; pad={}", "x".repeat(CREDENTIAL_LIMIT)));
    assert!(ResidentGrant::new_checked(&api, context(&scope, Instant::now()),
        &oversized, observation(&actual_id, &target)).is_err());
    close(&client, &scope).await;
}

#[tokio::test]
async fn cancellation_refuses_bearer_and_token_without_auth_http() {
    let (client, scope) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = bound(&scope, server.local_addr().unwrap(), NativeEgress::Direct);
    let checked = grant(&api, &scope, Instant::now());
    let retained = retained_fixture(&api, &scope);
    let expected = scope.clone();
    let pending = client.prepare_owned(&scope, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            control.cancel();
            let result = api.resident_bearer(checked, &control).await;
            assert!(result.cleanup_confirmed);
            assert!(matches!(result.value, Err(PreparationFailure::Cancelled)));
            let result = api.authorize_resident_token(retained, &session,
                observation(&identity(&expected), &target()),
                CastleToken("fixture-token".into()), &control);
            assert!(matches!(result, Err(PreparationFailure::Cancelled)));
            PreparationResult::finished(Ok(()))
        }).unwrap();
    assert!(matches!(tokio::time::timeout(BOUND, pending).await.unwrap(),
        Err(PreparationFailure::Cancelled)));
    assert!(server.accept().now_or_never().is_none());
    close(&client, &scope).await;
}

#[tokio::test]
async fn another_bound_client_cannot_use_checked_grant_or_retained_bearer() {
    let (client, account) = scope(NativeEgress::Direct);
    let (other_client, other) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = bound(&account, server.local_addr().unwrap(), NativeEgress::Direct);
    let wrong = bound(&other, server.local_addr().unwrap(), NativeEgress::Direct);
    let checked = grant(&api, &account, Instant::now());
    let retained = retained_fixture(&api, &account);
    let expected = account.clone();
    let pending = client.prepare_owned(&account, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            let result = wrong.resident_bearer(checked, &control).await;
            assert!(result.cleanup_confirmed);
            assert!(matches!(result.value, Err(PreparationFailure::Rejected)));
            let result = wrong.authorize_resident_token(retained, &session,
                observation(&identity(&expected), &target()),
                CastleToken("fixture-token".into()), &control);
            assert!(matches!(result, Err(PreparationFailure::Rejected)));
            PreparationResult::finished(Ok(()))
        }).unwrap();
    tokio::time::timeout(BOUND, pending).await.unwrap().unwrap();
    assert!(server.accept().now_or_never().is_none());
    close(&client, &account).await;
    close(&other_client, &other).await;
}
