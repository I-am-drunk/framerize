use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Case { Ordinary, ResidentAccepted, ResidentRejected, Cancelled }

async fn lifetime_case(seconds: i64, case: Case) {
    let (client, scope) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = bound(&scope, server.local_addr().unwrap(), NativeEgress::Direct);
    let checked = grant(&api, &scope, Instant::now());
    let expected = scope.clone();
    let (entered, entry) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn(async move {
        tokio::time::timeout(BOUND, async move {
            let (mut socket, _) = server.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            entered.send(()).unwrap();
            released.await.unwrap();
            let mut body = token();
            body["expiresAt"] = serde_json::json!((chrono::Utc::now()
                + chrono::Duration::seconds(seconds)).to_rfc3339());
            socket.write_all(response(200, &body.to_string(), "").as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
            (server, request)
        }).await.unwrap()
    });
    let pending = client.prepare_owned(&scope,
        tokio::time::Instant::now() + Duration::from_secs(45),
        move |session, control| async move {
            if case == Case::Ordinary {
                assert_eq!(api.bearer(&expected, &session).await.unwrap(), "fixture-bearer");
            } else {
                let outcome = api.resident_bearer(checked, &control).await;
                assert!(outcome.cleanup_confirmed);
                match case {
                    Case::ResidentAccepted => {
                        let bearer = outcome.value.unwrap();
                        assert!(bearer.as_str() == "fixture-bearer");
                        assert!(bearer.grant.current_header == JAR);
                    }
                    Case::ResidentRejected => assert!(matches!(outcome.value,
                        Err(PreparationFailure::Rejected))),
                    Case::Cancelled => assert!(matches!(outcome.value,
                        Err(PreparationFailure::Cancelled))),
                    Case::Ordinary => unreachable!(),
                }
            }
            PreparationResult::finished(Ok(()))
        }).unwrap();
    tokio::time::timeout(BOUND, entry).await.unwrap().unwrap();
    if case == Case::Cancelled { pending.cancellation().cancel(); }
    release.send(()).unwrap();
    let result = tokio::time::timeout(BOUND, pending).await.unwrap();
    if case == Case::Cancelled {
        assert!(matches!(result, Err(PreparationFailure::Cancelled)));
    } else { result.unwrap(); }
    let (server, request) = tokio::time::timeout(BOUND, worker).await.unwrap().unwrap();
    let cookie = if case == Case::Ordinary { session().cookie_header } else { JAR.into() };
    assert!(request.lines().any(|line| line.split_once(':').is_some_and(|(name, value)|
        name.eq_ignore_ascii_case("cookie") && value.trim_start() == cookie)));
    assert!(server.accept().now_or_never().is_none());
    close(&client, &scope).await;
}

#[tokio::test]
async fn resident_short_lifetime_refuses_after_one_settled_auth() {
    lifetime_case(35, Case::ResidentRejected).await;
}

#[tokio::test]
async fn resident_sufficient_lifetime_retains_the_same_bearer_and_jar() {
    lifetime_case(600, Case::ResidentAccepted).await;
}

#[tokio::test]
async fn ordinary_bearer_keeps_the_existing_minimum_lifetime() {
    lifetime_case(35, Case::Ordinary).await;
}

#[tokio::test]
async fn cancellation_after_auth_preserves_settlement_before_lifetime_check() {
    lifetime_case(35, Case::Cancelled).await;
}
