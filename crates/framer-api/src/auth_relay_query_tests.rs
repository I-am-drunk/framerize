//! Inert relay URL fixtures: requests are built, never sent.
use super::relay_request;

const NONCE: i64 = 123;

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

fn query_pairs(request: &reqwest::Request) -> Vec<(String, String)> {
    request.url().query_pairs().into_owned().collect()
}

#[test]
fn queryless_relay_preserves_endpoint_and_adds_nonce() {
    let request = relay_request(&client(), "https://fixture.invalid/tokens.json", NONCE)
        .build()
        .unwrap();
    assert_eq!(request.url().path(), "/tokens.json");
    assert_eq!(request.url().scheme(), "https");
    assert_eq!(request.url().host_str(), Some("fixture.invalid"));
    assert_eq!(request.url().query(), Some("r=123"));
    assert_eq!(query_pairs(&request), vec![("r".into(), "123".into())]);
    assert_eq!(request.url().fragment(), None);
    assert_eq!(request.method(), reqwest::Method::GET);
    assert!(request.body().is_none());
}

#[test]
fn existing_query_and_encoding_are_preserved() {
    let relay = "https://fixture.invalid/tokens%20archive.json?ref=feature%2Ffixture&name=two%20words&empty=&r=old";
    let request = relay_request(&client(), relay, NONCE).build().unwrap();
    assert_eq!(request.url().path(), "/tokens%20archive.json");
    assert_eq!(request.url().host_str(), Some("fixture.invalid"));
    assert_eq!(
        request.url().query(),
        Some("ref=feature%2Ffixture&name=two%20words&empty=&r=old&r=123")
    );
    assert_eq!(
        query_pairs(&request),
        vec![
            ("ref".into(), "feature/fixture".into()),
            ("name".into(), "two words".into()),
            ("empty".into(), "".into()),
            ("r".into(), "old".into()),
            ("r".into(), "123".into()),
        ]
    );
    assert_eq!(request.url().fragment(), None);
    assert_eq!(request.method(), reqwest::Method::GET);
}

#[test]
fn fragment_does_not_hide_nonce() {
    let client = client();
    for (relay, query) in [
        ("https://fixture.invalid/tokens.json#bookmark", "r=123"),
        (
            "https://fixture.invalid/tokens.json?ref=feature%2Ffixture#bookmark",
            "ref=feature%2Ffixture&r=123",
        ),
    ] {
        let request = relay_request(&client, relay, NONCE).build().unwrap();
        let nonces: Vec<_> = query_pairs(&request)
            .into_iter()
            .filter(|(key, _)| key == "r")
            .map(|(_, value)| value)
            .collect();
        assert_eq!(nonces, vec!["123".to_owned()], "{relay}");
        assert_eq!(request.url().query(), Some(query));
        assert_eq!(request.url().path(), "/tokens.json");
        assert_eq!(request.url().host_str(), Some("fixture.invalid"));
        assert_eq!(request.url().fragment(), Some("bookmark"));
        assert_eq!(request.method(), reqwest::Method::GET);
    }
}

#[test]
fn invalid_url_remains_builder_error() {
    let client = client();
    let relay = "http://[invalid";
    let original = client.get(relay).build().unwrap_err();
    // The helper still returns a builder, so headers can be attached before
    // the malformed URL is reported by build (or send in production).
    let pending = relay_request(&client, relay, NONCE)
        .header("accept", "application/vnd.github.raw")
        .header("user-agent", "framerize-relay/0.1");
    let error = pending.build().unwrap_err();
    assert!(error.is_builder());
    assert_eq!(error.to_string(), original.to_string());
    assert_eq!(
        std::error::Error::source(&error).unwrap().to_string(),
        std::error::Error::source(&original).unwrap().to_string()
    );
}
