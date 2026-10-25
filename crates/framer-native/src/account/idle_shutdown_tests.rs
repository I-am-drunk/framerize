use super::*;

#[tokio::test]
async fn idle_shutdown_joins_the_actual_account_supervisor() {
    let client = NativeAccountClient::new().unwrap();
    let task = client.supervisor_for_test();
    client.shutdown_idle().await.unwrap();
    assert!(task.is_finished());
    assert!(client.shutdown_idle().await.is_err());
}

#[tokio::test]
async fn idle_shutdown_rejects_open_accounts_until_their_real_close() {
    let client = NativeAccountClient::new().unwrap();
    let task = client.supervisor_for_test();
    let scope = client.open_account(AccountId::new(), Session {
        active_user: FramerUserId("shutdown-fixture".into()),
        cookie_header: "session=shutdown-fixture".into(),
    }, NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    assert!(matches!(client.shutdown_idle().await, Err(ClientError::NotReady)));
    assert!(!task.is_finished());
    client.close_account(&scope).unwrap().await.unwrap();
    client.shutdown_idle().await.unwrap();
    assert!(task.is_finished());
}
