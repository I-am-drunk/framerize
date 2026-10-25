use super::*;

fn account() -> AccountId { AccountId(uuid::Uuid::new_v4()) }

#[test]
fn sdk_timestamp_survives_cleanup_delay_without_refresh() {
    let handoff = Instant::now() - Duration::from_secs(10);
    let token = FreshCastleToken::from_sdk_output_at(account(),
        CastleToken("fixture-token".into()), handoff).unwrap();
    assert_eq!(token.received_at, handoff);
}

#[test]
fn future_sdk_handoff_is_rejected() {
    assert!(FreshCastleToken::from_sdk_output_at(account(),
        CastleToken("fixture-token".into()), Instant::now() + Duration::from_secs(30)).is_err());
}

#[test]
fn old_sdk_handoff_fails_the_existing_pre_send_age_check() {
    let account = account();
    let user = FramerUserId("fixture-user".into());
    let mut limits = NativeHttpLimits::default();
    limits.token_max_age = Duration::from_secs(1);
    let client = NativeHttp::new(account, user.clone(), limits, NativeEgress::Direct).unwrap();
    let session = Session { active_user: user, cookie_header: "session=fixture".into() };
    let token = FreshCastleToken::from_sdk_output_at(account,
        CastleToken("fixture-token".into()), Instant::now() - Duration::from_secs(2)).unwrap();
    let auth = AccountRequestAuth::new(account, &session, "fixture-bearer");
    assert_eq!(client.validate_request(&account, "/ai/v3/responses/", &auth, &token)
        .unwrap_err(), "native-http: fresh Castle token required");
}
