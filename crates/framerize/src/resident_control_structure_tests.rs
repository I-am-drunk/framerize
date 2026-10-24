use super::*;

// Existing synthetic empty-group vector, never a live token.
const EMPTY: &str = "Qk9DenHMFWL2aD3uCKum3zzBKEFts7Ki9Wden-AvMZcd-pq37k6jgh_DxxjeeXIfMgCMIU-BMA83WTr8HhQdAoQrUmjQQkPRb_-SUs3U094";
fn lease() -> uuid::Uuid { uuid::Uuid::from_u128(1) }

#[test]
fn absent_lease_emits_nothing() {
    assert!(facts(EMPTY, uuid::Uuid::nil()).is_none());
}

#[test]
fn malformed_token_exports_unavailable_without_input() {
    let fields = facts("private-fixture-input", lease()).unwrap();
    assert_eq!(fields["available"], false);
    assert_eq!(fields["structure"], serde_json::Value::Null);
    assert!(!serde_json::to_string(&fields).unwrap().contains("private-fixture-input"));
    assert_eq!(fields.len(), 4);
}

#[test]
fn valid_structure_is_bound_to_native_phase_and_original_lease() {
    let fields = facts(EMPTY, lease()).unwrap();
    assert_eq!(fields["available"], true);
    assert_eq!(fields["phase"], "native");
    assert_eq!(fields["lease"], lease().to_string());
    assert_eq!(fields["structure"], serde_json::json!({"entries": [], "consumed": 10}));
    assert_eq!(fields.len(), 4);
    assert!(!serde_json::to_string(&fields).unwrap().contains(EMPTY));
}
