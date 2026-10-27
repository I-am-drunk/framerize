use crate::account::{client::AccountScope, device::{DeviceAuthority, DeviceGrant}};
use crate::authorization::{control::OfflineMintTarget, CurrentRequestCookies};
use crate::transport::http::NativeEgress;
use framer_core::{ids::{AccountId, FramerUserId}, port::Session};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};

pub(super) const MAX_FRAME: usize = 128 * 1024;

/// Private exact context. No Debug implementation and no credential logging.
#[derive(Clone, Serialize)]
pub struct BrowserControlBinding {
    pub account: AccountId,
    pub user: FramerUserId,
    pub session: String,
    pub device: String,
    pub page_url: String,
    pub wrapper_url: String,
    pub post_path: String,
    pub proxy: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Grant {
    pub account: AccountId,
    pub native_generation: u64,
    pub host_generation: String,
    pub lease: String,
    pub device: String,
    pub authority: String,
}

impl BrowserControlBinding {
    pub(super) fn valid(&self) -> bool {
        let strings = [&self.user.0, &self.session, &self.device, &self.page_url,
            &self.wrapper_url, &self.post_path];
        let session = Session { active_user: self.user.clone(),
            cookie_header: format!("session={}", self.session) };
        strings.iter().all(|s| !s.is_empty() && s.len() <= 16 * 1024)
            && !self.account.0.is_nil() && CurrentRequestCookies::valid_session(&session)
            && crate::account::device::valid_device_id(&self.device)
            && self.proxy.as_ref().is_none_or(|s| !s.is_empty() && s.len() <= 16 * 1024)
            && reqwest::Url::parse(&self.page_url).is_ok_and(|u|
                u.scheme() == "https" && u.host_str() == Some("framer.com"))
            && reqwest::Url::parse(&self.wrapper_url).is_ok_and(|u| u.scheme() == "https"
                && u.host_str().is_some() && u.username().is_empty()
                && u.password().is_none() && u.fragment().is_none())
    }

    pub(super) fn matches(&self, scope: &AccountScope, session: &Session,
        grant: &DeviceGrant, target: &OfflineMintTarget) -> bool {
        let route = self.proxy.as_deref().map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct);
        self.account == scope.account() && self.account == grant.account
            && &self.user == scope.active_user() && self.user == session.active_user
            && self.device == grant.device && self.page_url == target.page_url()
            && self.post_path == target.path() && scope.matches_egress(&route)
            && CurrentRequestCookies::valid_session(session)
            && session.cookie_header.split(';').any(|part|
                part.trim().strip_prefix("session=") == Some(self.session.as_str()))
    }
}

impl Grant {
    pub(super) fn new(scope: &AccountScope, grant: &DeviceGrant) -> Option<Self> {
        if grant.lease.is_nil() || scope.generation() == 0 || grant.account != scope.account()
            || !generation_valid(&grant.host_generation) { return None; }
        Some(Self { account: grant.account, native_generation: scope.generation(),
            host_generation: grant.host_generation.clone(), lease: grant.lease.to_string(),
            device: grant.device.clone(), authority: match grant.authority {
                DeviceAuthority::Ledger => "ledger", DeviceAuthority::Observation => "observation",
            }.into() })
    }
    pub(super) fn matches(&self, grant: &DeviceGrant) -> bool {
        self.account == grant.account && self.host_generation == grant.host_generation
            && self.lease == grant.lease.to_string() && self.device == grant.device
            && self.authority == match grant.authority {
                DeviceAuthority::Ledger => "ledger", DeviceAuthority::Observation => "observation",
            }
    }
}

pub(super) fn generation_valid(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Reason { Finished, Cancelled, Deadline, Rejected, Unconfirmed }

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Reply {
    Ready { version: u8, host_generation: String, incarnation: u64, proxy_revision: u64 },
    Minted { version: u8, grant: Grant, token: String },
    Closed { version: u8, grant: Option<Grant>, cleanup_confirmed: bool, reason: Reason },
}

pub(super) async fn read(reader: &mut BufReader<ChildStdout>, buffer: &mut Vec<u8>)
    -> Result<Reply, ()> {
    let remaining = (MAX_FRAME + 1).checked_sub(buffer.len()).ok_or(())?;
    (&mut *reader).take(remaining as u64).read_until(b'\n', buffer).await.map_err(|_| ())?;
    if buffer.last() != Some(&b'\n') || buffer.len() > MAX_FRAME + 1 { return Err(()); }
    let value = serde_json::from_slice(buffer).map_err(|_| ())?;
    buffer.clear();
    Ok(value)
}

pub(super) async fn write(input: &mut ChildStdin, value: &serde_json::Value) -> Result<(), ()> {
    let mut frame = serde_json::to_vec(value).map_err(|_| ())?;
    if frame.len() > MAX_FRAME { return Err(()); }
    frame.push(b'\n');
    input.write_all(&frame).await.map_err(|_| ())?;
    input.flush().await.map_err(|_| ())
}
