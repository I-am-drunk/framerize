use serde_json::Value;
use tokio_tungstenite::tungstenite::http::Uri;

fn is_framer_page(url: &str) -> bool {
    let Ok(uri) = url.parse::<Uri>() else { return false; };
    uri.scheme_str().is_some_and(|scheme| scheme.eq_ignore_ascii_case("https"))
        && uri.authority().is_some_and(|authority| {
            let value = authority.as_str();
            value.eq_ignore_ascii_case("framer.com")
                || value.eq_ignore_ascii_case("framer.com:443")
        })
}

pub(super) fn websocket_url(targets: &Value) -> Option<&str> {
    targets.as_array()?.iter().find(|target| {
        target["type"].as_str() == Some("page")
            && target["url"].as_str().is_some_and(is_framer_page)
    }).and_then(|target| target["webSocketDebuggerUrl"].as_str())
}
