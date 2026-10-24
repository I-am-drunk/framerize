use super::*;

#[tokio::test]
async fn nonstream_unknown_cleanup_stays_fenced_after_body_drop_or_consumption() {
    enum Outcome { DropSuccess, ReadSuccess, ReadAdmissionError }
    for outcome in [Outcome::DropSuccess, Outcome::ReadSuccess, Outcome::ReadAdmissionError] {
        let mut f = fixture::Fixture::new();
        f.state = with_owned_operations(f.state, 32).unwrap();
        let ownership = f.state.operation_ownership.clone().unwrap();
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let blocked = if matches!(outcome, Outcome::ReadAdmissionError) {
            Some(f.state.gate.acquire(&f.key(1, "gpt-6-astra")).await)
        } else { None };
        let task = tokio::spawn(app.clone().oneshot(fixture_request(&f, false)));
        let call = f.next().await;
        let prefix = "host-lifecycle-unconfirmed: ";
        let error = format!("{prefix}{}é", "a".repeat(119 - prefix.len()));
        assert_eq!(error.find('é'), Some(119));
        assert_eq!(error.len(), 121);
        call.reply.send(Err(error)).ok().unwrap();
        let expected = if let Some(permit) = blocked {
            f.loaded(1).await;
            let mut cooling = f.accounts[1].clone();
            cooling.cooling_until_ms = Some(framer_core::rotation::now_ms() + 60_000);
            f.state.state.upsert(&cooling).await.unwrap();
            drop(permit);
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            // A later success proves only this call, not the lost cleanup.
            let next = f.next().await;
            assert_ne!(next.account, call.account);
            next.reply.send(Ok(fixture::success(next.kind))).ok().unwrap();
            StatusCode::OK
        };
        let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
        assert_eq!(response.status(), expected);
        if matches!(outcome, Outcome::ReadAdmissionError) {
            assert!(response.headers().contains_key(axum::http::header::RETRY_AFTER));
        }
        if !matches!(outcome, Outcome::DropSuccess) { to_bytes(response.into_body(), 65536).await.unwrap(); }
        else { drop(response); }
        assert!(ownership.gate.stats().unwrap().unconfirmed);
        let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
        assert!(snapshot.residents.iter().any(|slot|
            slot.account == call.account && slot.uncertain));
        let response = app.oneshot(fixture_request(&f, false)).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
            b"operation admission closed");
        f.no_call();
    }
}

#[tokio::test]
async fn disconnected_nonstream_waits_for_current_call_and_never_repicks() {
    for refused in [false, true] {
        let mut f = fixture::Fixture::new();
        f.state = with_owned_operations(f.state, 32).unwrap();
        let ownership = f.state.operation_ownership.clone().unwrap();
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let task = tokio::spawn(app.clone().oneshot(fixture_request(&f, false)));
        let call = f.next().await;
        task.abort(); let _ = task.await;
        assert!(!call.reply.is_closed());
        assert_eq!(ownership.gate.stats().unwrap().active, 2);
        assert!(!ownership.gate.stats().unwrap().unconfirmed);
        let reply = if refused { Err("risk-refused".into()) }
            else { Ok(fixture::success(call.kind)) };
        call.reply.send(reply).ok().unwrap();
        tokio::time::timeout(WAIT, async {
            while ownership.gate.stats().unwrap().active > 1 {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        f.no_call();
        assert!(!ownership.gate.stats().unwrap().unconfirmed);
        recovered_request(&mut f, app).await;
    }
}

async fn recovered_request(f: &mut fixture::Fixture, app: Router) {
    let mut request = fixture_request(f, false);
    request.headers_mut().insert("x-framerize-account",
        f.accounts[1].id.0.to_string().parse().unwrap());
    let task = tokio::spawn(app.oneshot(request));
    let call = f.next().await;
    call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
    let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("fixture"));
}

#[tokio::test]
async fn dropping_completed_nonstream_response_preserves_admission() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let task = tokio::spawn(app.clone().oneshot(fixture_request(&f, false)));
    let call = f.next().await;
    call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
    let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(ownership.gate.stats().unwrap().active, 2);
    drop(response);
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
    recovered_request(&mut f, app).await;
}
