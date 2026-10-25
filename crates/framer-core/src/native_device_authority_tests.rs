use super::*;
use serde_json::{json, Value};

fn reply(lease: uuid::Uuid, authority: Value) -> Value {
    json!({"generation": "0123456789abcdef0123456789abcdef", "lease": lease,
        "device": "authority-fixture", "authority": authority})
}

#[test]
fn negotiated_device_authority_preserves_both_modes() {
    let account = AccountId(uuid::Uuid::new_v4());
    let lease = uuid::Uuid::new_v4();
    for (wire, expected) in [("ledger", DeviceAuthority::Ledger),
        ("observation", DeviceAuthority::Observation)] {
        let grant = DeviceGrant::from_wire(account, lease, reply(lease, json!(wire))).unwrap();
        assert_eq!(grant.authority, expected);
        assert_eq!(grant.account, account);
        assert_eq!(grant.lease, lease);
        assert_eq!(grant.device, "authority-fixture");
    }
}

#[test]
fn negotiated_device_authority_rejects_unknown_or_untyped_modes() {
    let account = AccountId(uuid::Uuid::new_v4());
    let lease = uuid::Uuid::new_v4();
    for mode in [Value::Null, json!(true), json!(1), json!(""), json!("other")] {
        assert!(matches!(DeviceGrant::from_wire(account, lease, reply(lease, mode)),
            Err(DeviceFailure::Unconfirmed)));
    }
}

#[test]
fn negotiated_device_authority_does_not_infer_legacy_mode() {
    let account = AccountId(uuid::Uuid::new_v4());
    let lease = uuid::Uuid::new_v4();
    let mut value = reply(lease, json!("ledger"));
    value.as_object_mut().unwrap().remove("authority");
    assert!(matches!(DeviceGrant::from_wire(account, lease, value),
        Err(DeviceFailure::Unconfirmed)));
}

#[test]
fn negotiated_device_authority_remains_bound_to_the_open_reply() {
    let account = AccountId(uuid::Uuid::new_v4());
    let lease = uuid::Uuid::new_v4();
    for (field, value) in [("lease", json!(uuid::Uuid::new_v4())),
        ("generation", json!("foreign")), ("device", json!("invalid device")),
        ("extra", json!("observation"))] {
        let mut changed = reply(lease, json!("observation"));
        changed[field] = value;
        assert!(matches!(DeviceGrant::from_wire(account, lease, changed),
            Err(DeviceFailure::Unconfirmed)));
    }
}
