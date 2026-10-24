use super::*;

#[tokio::test]
async fn missing_v2_config_cannot_fall_back_to_ordinary_serving() {
    let (state, ports, _, _) = fixture();
    let path = std::env::temp_dir().join(format!("missing-v2-config-{}.json", uuid::Uuid::new_v4()));
    let result = crate::offline_http::configure_v2(state, path.to_str(), None);
    assert!(matches!(result, Err("cannot open offline runtime config")));
    assert!(ports.events.lock().unwrap().is_empty());
    assert!(!path.exists());
}

#[tokio::test]
async fn explicit_v2_without_worker_config_refuses_before_store_or_browser() {
    let (state, ports, _, _) = fixture();
    let result = crate::offline_http::configure_v2(state, None, None);
    assert!(matches!(result, Err("V2 serving requires its worker config")));
    assert!(ports.events.lock().unwrap().is_empty());
}
