use super::{connect_async, WsStream};
use std::time::Duration;

pub(super) async fn connect(url: &str) -> Result<WsStream, String> {
    // The future owns TCP and the upgrade handshake, including cancellation.
    tokio::time::timeout(Duration::from_secs(5), async {
        connect_async(url).await
            .map(|(socket, _)| socket)
            .map_err(|error| error.to_string())
    }).await.map_err(|_| "devtools WebSocket connection timed out".to_owned())?
}
