// Source-only inert fixtures. No live account, host, browser or upstream calls.
use super::*;
use serde_json::json;
use std::{fs, os::unix::fs::{symlink, MetadataExt, PermissionsExt}, path::PathBuf};

fn input() -> Value {
    json!({"account_id":"00000000-0000-0000-0000-000000000001",
        "user_id":"fixture-user", "team_id":"fixture-team", "project_id":"fixture-project",
        "conversation_id":"fixture-conversation", "route":"/ai/v3/responses/",
        "feature_category":"fixture-category", "feature_name":"fixture-feature",
        "step":"fixture-step", "turn_id":"fixture-native", "baseline_turn_id":"fixture-browser",
        "user_prompt":"private-prompt-marker", "timeout_ms":3000, "token_limit":32,
        "request":{"model":"gpt-4.1", "input":"private-input-marker", "stream":false}})
}
fn parsed(value: &Value) -> io::Result<(Specification, String)> {
    parse(&serde_json::to_vec(value).unwrap(), Instant::now())
}

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let root = fs::canonicalize(std::env::temp_dir()).unwrap()
            .join(format!("resident-socket-fixture-{}", uuid::Uuid::new_v4()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        Self(root)
    }
    fn socket(&self) -> PathBuf { self.0.join("control.sock") }
}
impl Drop for Temp {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

#[test]
fn peer_identity_requires_exact_effective_uid() {
    assert!(platform::same_peer(1000, 1000));
    assert!(!platform::same_peer(1000, 1001));
    assert!(!platform::same_peer(1000, 0));
}

#[test]
fn parent_metadata_requires_ownership_and_exact_mode() {
    let temp = Temp::new(); let metadata = fs::metadata(&temp.0).unwrap();
    assert!(platform::private_parent(&metadata, metadata.uid()));
    assert!(!platform::private_parent(&metadata, metadata.uid().wrapping_add(1)));
    fs::set_permissions(&temp.0, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(!platform::private_parent(&fs::metadata(&temp.0).unwrap(), metadata.uid()));
}

#[test]
fn every_context_field_is_required_without_defaults() {
    for key in input().as_object().unwrap().keys() {
        let mut value = input();
        value.as_object_mut().unwrap().remove(key);
        assert!(parsed(&value).is_err(), "missing {key}");
    }
}

#[test]
fn only_two_free_routes_are_supported() {
    let mut value = input();
    value["route"] = json!("/ai/v3/messages/");
    value["request"] = json!({"model":"gpt-4.1", "stream":false,
        "messages":[{"role":"user", "content":"fixture"}]});
    assert!(matches!(parsed(&value).unwrap().0.route, FreeRoute::Messages));
    for route in ["responses", "/ai/v3/openai/responses/", "/ai/v3/responses", "https://x/"] {
        value["route"] = json!(route);
        assert!(parsed(&value).is_err());
    }
}

#[test]
fn extra_top_level_context_is_rejected() {
    for key in ["page_url", "device_id", "provenance", "account", "retry", "unexpected"] {
        let mut value = input(); value[key] = json!("private-marker");
        assert!(parsed(&value).is_err());
    }
}

#[test]
fn structured_credentials_are_refused_at_any_depth() {
    for key in ["Authorization", "cookie_header", "castleToken", "api-key", "password", "headers"] {
        let mut value = input();
        value["request"]["nested"] = json!([{(key):"private-secret-marker"}]);
        let error = parsed(&value).err().unwrap().to_string();
        assert_eq!(error, "private resident control refused");
        assert!(!error.contains("private-secret-marker"));
    }
}

#[test]
fn duplicate_keys_and_escaped_credential_names_are_refused() {
    for wire in [r#"{"x":1,"x":2}"#, r#"{"nested":{"x":1,"x":2}}"#,
        r#"{"\u0061uthorization":"private-secret-marker"}"#] {
        assert!(serde_json::from_str::<StrictJson>(wire).is_err());
    }
}

#[test]
fn malformed_json_trailing_documents_and_oversize_are_refused() {
    for wire in [b"".as_slice(), b"{", b"null", b"[]", b"{} {}", b"{\"x\":NaN}", b"\xff"] {
        assert!(parse(wire, Instant::now()).is_err());
    }
    assert!(parse(&vec![b' '; MAX_PAYLOAD + 1], Instant::now()).is_err());
    let deep = format!("{}0{}", "[".repeat(140), "]".repeat(140));
    assert!(serde_json::from_str::<StrictJson>(&deep).is_err());
}

#[test]
fn plan_validation_rejects_streaming_and_invalid_payloads() {
    for (key, replacement) in [("stream", json!(true)), ("model", json!("")),
        ("input", json!(null)), ("previous_response_id", json!("old")),
        ("background", json!(false)), ("store", json!(false))] {
        let mut value = input(); value["request"][key] = replacement;
        assert!(parsed(&value).is_err(), "field {key}");
    }
}

#[test]
fn identity_turn_and_numeric_bounds_are_enforced() {
    for (key, replacement) in [("account_id", json!(uuid::Uuid::nil())),
        ("project_id", json!("")), ("user_id", json!("bad\nuser")),
        ("team_id", json!("x".repeat(4097))), ("conversation_id", json!("")),
        ("baseline_turn_id", json!("fixture-native")), ("baseline_turn_id", json!("")),
        ("baseline_turn_id", json!("x".repeat(257))), ("turn_id", json!("")),
        ("timeout_ms", json!(0)), ("timeout_ms", json!(60001)),
        ("timeout_ms", json!(-1)), ("token_limit", json!(15)), ("token_limit", json!(513))] {
        let mut value = input(); value[key] = replacement;
        assert!(parsed(&value).is_err(), "field {key}");
    }
}

#[test]
fn serialization_contains_only_fixed_report_fields() {
    let report = Report { baseline_accepted: true, baseline_status: Some(200),
        status: Some(403), ..Report::default() };
    let value = serde_json::to_value(facts(&report)).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 13);
    assert_eq!(value["baseline_accepted"], true);
    assert_eq!(value["status"], 403);
}

#[test]
fn cleanup_join_cannot_upgrade_unconfirmed_report() {
    let report = join_cleanup(Report::default(), true);
    assert!(!report.cleanup_confirmed);
    let report = join_cleanup(preflight(Failure::Admission), false);
    assert!(!report.cleanup_confirmed);
    assert_eq!(report.failure, Some(Failure::Cleanup));
}

#[test]
fn refusal_and_failure_names_are_fixed_facts() {
    use framer_api::classify::{Refusal, RefusalBasis, RefusalObservation};
    let report = Report { failure: Some(Failure::Native),
        refusal: Some(RefusalObservation { policy: Refusal::Castle,
            basis: RefusalBasis::FramerSecurityEnvelope }), ..Report::default() };
    let value = serde_json::to_value(facts(&report)).unwrap();
    assert_eq!(value["failure"], "native");
    assert_eq!(value["refusal_policy"], "castle");
    assert_eq!(value["refusal_basis"], "framer_security_envelope");
    for field in ["request", "reply", "token", "headers", "body", "account_id", "user_prompt"] {
        assert!(value.get(field).is_none());
    }
}

#[tokio::test]
async fn zero_and_oversize_prefixes_refuse_before_body_allocation() {
    for length in [0, MAX_PAYLOAD as u32 + 1, u32::MAX] {
        let (mut writer, mut reader) = tokio::io::duplex(4);
        writer.write_u32(length).await.unwrap();
        assert!(read_frame(&mut reader, Instant::now() + READ_WINDOW).await.is_err());
    }
}

#[tokio::test]
async fn truncated_prefix_and_body_refuse() {
    for bytes in [vec![0, 0], vec![0, 0, 0, 3, b'{']] {
        let (mut writer, mut reader) = tokio::io::duplex(32);
        writer.write_all(&bytes).await.unwrap(); writer.shutdown().await.unwrap();
        assert!(read_frame(&mut reader, Instant::now() + READ_WINDOW).await.is_err());
    }
}

#[tokio::test]
async fn read_deadline_is_not_extended_by_partial_input() {
    let (mut writer, mut reader) = tokio::io::duplex(4);
    writer.write_all(&[0, 0]).await.unwrap();
    assert!(read_frame(&mut reader, Instant::now()).await.is_err());
}

#[tokio::test]
async fn exact_frame_cap_and_big_endian_prefix_are_supported() {
    let (mut writer, mut reader) = tokio::io::duplex(MAX_FRAME);
    writer.write_u32(MAX_PAYLOAD as u32).await.unwrap();
    writer.write_all(&vec![b' '; MAX_PAYLOAD]).await.unwrap();
    assert_eq!(read_frame(&mut reader, Instant::now() + READ_WINDOW).await.unwrap().len(), MAX_PAYLOAD);
}

#[tokio::test]
async fn a_second_frame_is_never_consumed_as_part_of_the_first() {
    let (mut writer, mut reader) = tokio::io::duplex(32);
    writer.write_all(&[0, 0, 0, 2, b'{', b'}', 0, 0, 0, 2, b'{', b'}']).await.unwrap();
    assert_eq!(read_frame(&mut reader, Instant::now() + READ_WINDOW).await.unwrap(), b"{}");
    let mut extra = [0];
    assert_eq!(reader.read(&mut extra).await.unwrap(), 1);
}

#[tokio::test]
async fn report_uses_one_length_prefixed_json_frame_then_eof() {
    let (mut writer, mut reader) = tokio::io::duplex(2048);
    write_report(&mut writer, &preflight(Failure::Context)).await;
    let bytes = read_frame(&mut reader, Instant::now() + READ_WINDOW).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["failure"], "context");
    assert_eq!(value["cleanup_confirmed"], true);
    assert_eq!(reader.read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test]
async fn bind_is_private_and_drop_removes_its_own_socket() {
    let temp = Temp::new(); let path = temp.socket();
    let bound = BoundSocket::bind(&path).unwrap();
    let metadata = fs::symlink_metadata(&path).unwrap();
    assert_eq!(metadata.mode() & 0o7777, 0o600);
    assert_eq!(metadata.uid(), fs::metadata(&temp.0).unwrap().uid());
    assert!(platform::own_socket(&metadata, platform::ObjectId::of(&metadata)));
    drop(bound);
    assert!(!path.exists());
}

#[tokio::test]
async fn bind_never_creates_parent_or_accepts_relative_paths() {
    let temp = Temp::new(); let missing = temp.0.join("missing");
    assert!(BoundSocket::bind(&missing.join("socket")).is_err());
    assert!(!missing.exists());
    assert!(BoundSocket::bind(Path::new("relative.sock")).is_err());
}

#[tokio::test]
async fn bind_refuses_existing_file_and_dangling_symlink() {
    let temp = Temp::new(); let path = temp.socket();
    fs::write(&path, b"preserve").unwrap();
    assert!(BoundSocket::bind(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"preserve");
    fs::remove_file(&path).unwrap(); symlink("missing", &path).unwrap();
    assert!(BoundSocket::bind(&path).is_err());
    assert_eq!(fs::read_link(&path).unwrap(), Path::new("missing"));
}

#[tokio::test]
async fn existing_socket_is_never_taken_over() {
    let temp = Temp::new(); let path = temp.socket();
    let bound = BoundSocket::bind(&path).unwrap();
    let id = platform::ObjectId::of(&fs::symlink_metadata(&path).unwrap());
    assert!(BoundSocket::bind(&path).is_err());
    assert!(platform::own_socket(&fs::symlink_metadata(&path).unwrap(), id));
    drop(bound);
}

#[tokio::test]
async fn symlinked_parent_and_permissive_directory_are_refused() {
    let temp = Temp::new(); let alias = temp.0.join("alias");
    symlink(&temp.0, &alias).unwrap();
    assert!(BoundSocket::bind(&alias.join("control.sock")).is_err());
    for mode in [0o755, 0o701, 0o770] {
        fs::set_permissions(&temp.0, fs::Permissions::from_mode(mode)).unwrap();
        assert!(BoundSocket::bind(&temp.socket()).is_err());
    }
    assert!(!temp.socket().exists());
}

#[tokio::test]
async fn cleanup_preserves_a_replacement_socket() {
    let temp = Temp::new(); let path = temp.socket();
    let bound = BoundSocket::bind(&path).unwrap();
    let old = platform::ObjectId::of(&fs::symlink_metadata(&path).unwrap());
    fs::remove_file(&path).unwrap();
    let replacement = std::os::unix::net::UnixListener::bind(&path).unwrap();
    assert!(!platform::own_socket(&fs::symlink_metadata(&path).unwrap(), old));
    drop(bound);
    assert!(path.exists()); drop(replacement);
}

#[tokio::test]
async fn cleanup_preserves_a_symlink_to_the_original_socket() {
    let temp = Temp::new(); let path = temp.socket();
    let bound = BoundSocket::bind(&path).unwrap(); let saved = temp.0.join("saved.sock");
    fs::rename(&path, &saved).unwrap(); symlink(&saved, &path).unwrap();
    drop(bound);
    assert_eq!(fs::read_link(&path).unwrap(), saved);
}

#[tokio::test]
async fn cleanup_uses_pinned_parent_after_a_directory_rename() {
    use std::os::unix::fs::DirBuilderExt;
    let temp = Temp::new(); let parent = temp.0.join("parent");
    fs::DirBuilder::new().mode(0o700).create(&parent).unwrap();
    let path = parent.join("socket"); let bound = BoundSocket::bind(&path).unwrap();
    let original = temp.0.join("original"); fs::rename(&parent, &original).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&parent).unwrap();
    fs::write(&path, b"preserve replacement").unwrap();
    drop(bound);
    assert!(!original.join("socket").exists());
    assert_eq!(fs::read(&path).unwrap(), b"preserve replacement");
}

#[test]
fn exact_identity_and_context_are_preserved() {
    let (spec, baseline) = parsed(&input()).unwrap();
    assert_eq!(spec.identity.account.0, uuid::Uuid::from_u128(1));
    assert_eq!(spec.identity.user.0, "fixture-user");
    assert_eq!(spec.identity.team.0, "fixture-team");
    assert_eq!(spec.identity.project.0, "fixture-project");
    assert_eq!(spec.identity.chat_session, "fixture-conversation");
    assert_eq!(spec.context.turn_id, "fixture-native");
    assert_eq!(spec.context.user_prompt, "private-prompt-marker");
    assert_eq!(baseline, "fixture-browser");
    assert!(matches!(spec.route, FreeRoute::Responses));
}
