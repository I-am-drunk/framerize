//! Private temporary records only; no browser or real account state is used.
use super::*;

struct Profile(PathBuf);

impl Profile {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cdp-device-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn record(&self) -> PathBuf { DeviceRecord::path(&self.0) }
}

impl Drop for Profile {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

#[tokio::test]
async fn malformed_existing_records_are_not_replaced() {
    let profile = Profile::new();
    for raw in ["{", "{}", "[]", "{\"castle_device_id\":null}", "{\"castle_device_id\":7}"] {
        std::fs::write(profile.record(), raw).unwrap();
        assert!(DeviceRecord::load_or_create(&profile.0).await.is_err());
        assert_eq!(std::fs::read_to_string(profile.record()).unwrap(), raw);
    }
}

#[tokio::test]
async fn unreadable_utf8_record_is_not_replaced() {
    let profile = Profile::new();
    let raw = b"{\"castle_device_id\":\"\xff\"}";
    std::fs::write(profile.record(), raw).unwrap();
    assert!(DeviceRecord::load_or_create(&profile.0).await.is_err());
    assert_eq!(std::fs::read(profile.record()).unwrap(), raw.to_vec());
}

#[tokio::test]
async fn existing_string_identity_and_unknown_bytes_are_preserved() {
    let profile = Profile::new();
    for id in ["saved-device", ""] {
        let raw = format!("{{ \"castle_device_id\": {id:?}, \"future\": [1, true] }}\n");
        std::fs::write(profile.record(), &raw).unwrap();
        assert_eq!(DeviceRecord::load_or_create(&profile.0).await.unwrap(), id);
        assert_eq!(std::fs::read_to_string(profile.record()).unwrap(), raw);
    }
}

#[tokio::test]
async fn missing_record_creates_one_reusable_identity() {
    let profile = Profile::new();
    let first = DeviceRecord::load_or_create(&profile.0).await.unwrap();
    assert!(!first.is_empty());
    let saved = std::fs::read(profile.record()).unwrap();
    let value: Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(value["castle_device_id"].as_str(), Some(first.as_str()));
    assert_eq!(DeviceRecord::load_or_create(&profile.0).await.unwrap(), first);
    assert_eq!(std::fs::read(profile.record()).unwrap(), saved);
}
