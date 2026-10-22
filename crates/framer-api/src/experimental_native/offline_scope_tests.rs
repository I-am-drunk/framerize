use super::*;
use crate::experimental_native::native_http::NativeHttpLimits;

#[tokio::test]
async fn joined_result_from_a_different_account_is_never_relabelled() {
    let (first, original) = scope(NativeEgress::Direct);
    let (second, other) = scope(NativeEgress::Direct);
    reject_result(&second, &other, &original).await;
    close(&first, &original).await;
    close(&second, &other).await;
}

#[tokio::test]
async fn joined_result_from_a_retired_generation_is_never_relabelled() {
    let (client, original) = scope(NativeEgress::Direct);
    close(&client, &original).await;
    let replacement = client.open_account(original.account(), fixture::session(),
        NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    assert_eq!(original.account(), replacement.account());
    assert_ne!(original.generation(), replacement.generation());
    reject_result(&client, &replacement, &original).await;
    close(&client, &replacement).await;
}

#[tokio::test]
async fn retired_minter_cannot_supply_a_new_generation_of_the_same_account() {
    let (client, original) = scope(NativeEgress::Direct);
    let minter = good(&original);
    close(&client, &original).await;
    let replacement = client.open_account(original.account(), fixture::session(),
        NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    assert_eq!(original.account(), replacement.account());
    assert_ne!(original.generation(), replacement.generation());
    let listener = listener().await;
    assert!(pair(&replacement, listener.local_addr().unwrap(), minter.clone()).is_err());
    assert_eq!(minter.calls.load(Ordering::SeqCst), 0);
    assert!(tokio::time::timeout(std::time::Duration::from_millis(20),
        listener.accept()).await.is_err());
    close(&client, &replacement).await;
}

async fn reject_result(client: &NativeAccountClient, requested: &AccountScope,
    provenance: &AccountScope) {
    let listener = listener().await;
    let minter = Arc::new(Minter::new(requested,
        PreparationResult::finished(Ok(minted(provenance, "fixture-token".into())))));
    let source = source(requested, listener.local_addr().unwrap(), minter.clone());
    let server = auth(listener);
    assert_eq!(prepare(client, requested, source, target()).await.unwrap_err(),
        PreparationFailure::Rejected);
    assert_eq!(minter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(minter.joined.load(Ordering::SeqCst), 1);
    fixture::result(server).await;
}

fn pair(account: &AccountScope, address: std::net::SocketAddr,
    minter: Arc<Minter>) -> Result<OfflineSessionSource, String> {
    OfflineSessionSource::new(
        Arc::new(fixture::bound(account, address, NativeEgress::Direct)),
        OfflineMintTarget::new(target(), "https://framer.com/projects/fixture-project".into())
            .unwrap(), minter)
}

#[tokio::test]
async fn same_target_cannot_pair_a_different_accounts_minter() {
    let (first, original) = scope(NativeEgress::Direct);
    let (second, other) = scope(NativeEgress::Direct);
    let listener = listener().await;
    let minter = good(&original);
    assert!(pair(&other, listener.local_addr().unwrap(), minter.clone()).is_err());
    assert_eq!(minter.calls.load(Ordering::SeqCst), 0);
    assert!(tokio::time::timeout(std::time::Duration::from_millis(20),
        listener.accept()).await.is_err());
    close(&first, &original).await;
    close(&second, &other).await;
}
