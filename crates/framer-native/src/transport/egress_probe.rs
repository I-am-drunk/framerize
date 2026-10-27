//! Credential-free point observation through the existing native HTTP client.
use super::{NativeCancellation, NativeHttp, Permit};
use std::{net::IpAddr, time::Duration};

const MAX_URL: usize = 4096;
const MAX_BODY: usize = 1024;
const MAX_READS: usize = 2048;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct IpReply { ip: IpAddr }

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EgressProbeError { Rejected, Cancelled, Deadline }

pub(crate) struct EgressProbeRequest {
    url: reqwest::Url,
    budget: Duration,
}

pub(crate) struct EgressProbeReply {
    pub url: String,
    pub address: IpAddr,
}

impl NativeHttp {
    pub(crate) async fn observe_egress(&self, request: EgressProbeRequest,
        cancellation: NativeCancellation) -> Result<EgressProbeReply, EgressProbeError> {
        use EgressProbeError::{Cancelled, Deadline, Rejected};
        if self.browser.is_some() { return Err(Rejected); }
        cancellation.claim().map_err(|_| Cancelled)?;
        let _permit = Permit::acquire(&self.in_flight, self.limits.in_flight)
            .map_err(|_| Rejected)?;
        let budget = request.budget;
        // Dropping the losing future drops its request/response. No task is
        // detached. The account's preparation worker observes this return.
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(Cancelled),
            result = tokio::time::timeout(budget, self.egress_once(request)) =>
                result.unwrap_or(Err(Deadline)),
        }
    }

    async fn egress_once(&self, request: EgressProbeRequest)
        -> Result<EgressProbeReply, EgressProbeError> {
        use EgressProbeError::Rejected;
        let mut response = self.client.get(request.url.clone())
            .header(reqwest::header::CACHE_CONTROL, "no-store")
            .send().await.map_err(|_| Rejected)?;
        let json = response.headers().get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()).and_then(|value| value.split(';').next())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"));
        if response.status() != reqwest::StatusCode::OK || response.url() != &request.url || !json {
            return Err(Rejected);
        }
        let mut body = Vec::with_capacity(MAX_BODY);
        for _ in 0..MAX_READS {
            match response.chunk().await.map_err(|_| Rejected)? {
                Some(bytes) if bytes.len() <= MAX_BODY - body.len() => body.extend_from_slice(&bytes),
                Some(_) => return Err(Rejected),
                None => {
                    let reply: IpReply = serde_json::from_slice(&body).map_err(|_| Rejected)?;
                    return Ok(EgressProbeReply { url: request.url.into(), address: reply.ip });
                }
            }
        }
        Err(Rejected)
    }
}

impl EgressProbeRequest {
    pub(crate) fn new(url: &str, budget: Duration) -> Result<Self, EgressProbeError> {
        Self::validated(url, budget, false)
    }

    fn validated(raw: &str, budget: Duration, loopback: bool) -> Result<Self, EgressProbeError> {
        use EgressProbeError::Rejected;
        if raw.len() > MAX_URL || !raw.is_ascii() || raw.bytes().any(|b| b <= 32 || b == 127)
            || raw.contains('\\') || budget.is_zero() || budget > Duration::from_secs(30) {
            return Err(Rejected);
        }
        let url = reqwest::Url::parse(raw).map_err(|_| Rejected)?;
        let local = loopback && url.scheme() == "http" && url.host_str()
            .and_then(|host| host.parse::<IpAddr>().ok()).is_some_and(|ip| ip.is_loopback());
        if (url.scheme() != "https" && !local) || url.host_str().is_none()
            || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some()
            || url.as_str() != raw {
            return Err(Rejected);
        }
        let mut nonce = url.query_pairs().filter(|(key, _)| key == "framerize_probe");
        let valid = nonce.next().is_some_and(|(_, value)| value.len() == 32
            && value.bytes().all(|b| b.is_ascii_hexdigit())) && nonce.next().is_none();
        if !valid { return Err(Rejected); }
        drop(nonce);
        Ok(Self { url, budget })
    }

    #[cfg(test)]
    pub(crate) fn for_loopback(url: &str, budget: Duration) -> Result<Self, EgressProbeError> {
        Self::validated(url, budget, true)
    }
}

#[cfg(test)]
#[path = "egress_probe_tests.rs"]
pub(crate) mod tests;
