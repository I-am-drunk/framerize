//! Explicit, credential-free launch inputs. Loading never starts a child.
use super::*;
use framer_core::ids::{AccountId, FramerUserId};
use serde::Deserialize;
use std::sync::{Arc, Mutex as SyncMutex};

const ERROR: &str = "invalid browser control launch descriptor";
const MAX_DESCRIPTOR: u64 = 64 * 1024;

#[cfg(test)]
#[path = "browser_control_descriptor_tests.rs"]
mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    version: u8,
    account: AccountId,
    user: FramerUserId,
    device: String,
    page_url: String,
    wrapper_url: String,
    post_path: String,
    python: PathBuf,
    python_sha256: String,
    script: PathBuf,
    script_sha256: String,
    state_directory: PathBuf,
    egress_endpoint: String,
    proxy_revision: u64,
    prepare_ms: u64,
    probe_ms: u64,
    witness_ms: u64,
}

#[derive(Default)]
struct DeferredState {
    used: bool,
    closed: bool,
    peer: Option<Arc<dyn BrowserCredentialControl>>,
}

struct ConfiguredControl { settings: Settings, state: SyncMutex<DeferredState> }

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl BrowserCredentialControl for ConfiguredControl {
    fn finish<'a>(&'a self, grant: &'a DeviceGrant, joined: bool, risk_refused: bool)
        -> BoxFuture<'a, bool> {
        async move {
            match self.peer(false) {
                Ok(Some(peer)) => peer.finish(grant, joined, risk_refused).await,
                _ => false,
            }
        }.boxed()
    }

    fn shutdown(&self) -> BoxFuture<'_, bool> {
        async move {
            let peer = match self.state.lock() {
                Ok(mut state) => {
                    state.closed = true;
                    state.peer.clone()
                },
                Err(_) => return false,
            };
            match peer {
                Some(peer) => peer.shutdown().await,
                None => true,
            }
        }.boxed()
    }

    fn requires_account_scope(&self) -> bool { true }
    fn prepare(&self) -> BoxFuture<'_, bool> { async { false }.boxed() }

    fn prepare_for_account<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        control: &'a PreparationControl) -> BoxFuture<'a, bool> {
        async move {
            if control.is_cancelled() { return false; }
            let Ok(peer) = self.install(scope, session) else { return false; };
            peer.prepare_for_account(scope, session, control).await
        }.boxed()
    }

    fn mint<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        grant: &'a DeviceGrant, target: &'a OfflineMintTarget, budget_ms: u64,
        control: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<BrowserControlMint>> {
        async move {
            match self.peer(true) {
                Ok(Some(peer)) => peer.mint(scope, session, grant, target, budget_ms, control).await,
                Ok(None) => PreparationResult::finished(Err(PreparationFailure::Rejected)),
                Err(()) => PreparationResult::unconfirmed(),
            }
        }.boxed()
    }
}

fn seconds(ms: u64) -> String { format!("{}.{:03}", ms / 1000, ms % 1000) }

impl ConfiguredControl {
    fn install(&self, scope: &AccountScope, session: &Session)
        -> Result<Arc<dyn BrowserCredentialControl>, &'static str> {
        let mut state = self.state.lock().map_err(|_| ERROR)?;
        if state.used || state.closed { return Err(ERROR); }
        state.used = true;
        let binding = self.settings.binding(scope, session)?;
        let mut peer = BrowserControlProcess::new(self.settings.launch(), binding)?
            .with_native_egress(&self.settings.egress_endpoint)?;
        peer.prepare_limit = Duration::from_millis(self.settings.prepare_ms);
        let peer: Arc<dyn BrowserCredentialControl> = Arc::new(peer);
        // Publish ownership before any async preparation can start a child.
        state.peer = Some(peer.clone());
        Ok(peer)
    }

    fn peer(&self, open_only: bool) -> Result<Option<Arc<dyn BrowserCredentialControl>>, ()> {
        let state = self.state.lock().map_err(|_| ())?;
        Ok(if open_only && state.closed { None } else { state.peer.clone() })
    }
}

fn read_descriptor(path: &Path, expected: &str) -> Result<Vec<u8>, &'static str> {
    use std::os::unix::fs::OpenOptionsExt;
    if !path.is_absolute() || !valid_hash(expected) { return Err(ERROR); }
    let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK)
        .open(path).map_err(|_| ERROR)?;
    let meta = file.metadata().map_err(|_| ERROR)?;
    if !meta.is_file() || meta.len() > MAX_DESCRIPTOR { return Err(ERROR); }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_DESCRIPTOR + 1).read_to_end(&mut bytes).map_err(|_| ERROR)?;
    if bytes.len() as u64 > MAX_DESCRIPTOR || format!("{:x}", Sha256::digest(&bytes)) != expected {
        return Err(ERROR);
    }
    Ok(bytes)
}

/// Read only the pinned JSON. Credentials are bound during owned preparation.
/// Imported Python modules still require the caller's immutable publication.
pub fn load_browser_control(path: &Path, expected_sha256: &str)
    -> Result<Arc<dyn BrowserCredentialControl>, &'static str> {
    let settings: Settings = serde_json::from_slice(&read_descriptor(path, expected_sha256)?)
        .map_err(|_| ERROR)?;
    settings.validate()?;
    Ok(Arc::new(ConfiguredControl { settings, state: SyncMutex::default() }))
}

impl Settings {
    fn launch(&self) -> BrowserControlLaunch {
        BrowserControlLaunch { python: self.python.clone(), python_sha256: self.python_sha256.clone(),
            script: self.script.clone(), script_sha256: self.script_sha256.clone(), arguments: vec![
                "--egress-endpoint".into(), self.egress_endpoint.clone().into(),
                "--state-directory".into(), self.state_directory.as_os_str().to_owned(),
                "--proxy-revision".into(), self.proxy_revision.to_string().into(),
                "--prepare-seconds".into(), seconds(self.prepare_ms).into(),
                "--probe-seconds".into(), seconds(self.probe_ms).into(),
                "--witness-seconds".into(), seconds(self.witness_ms).into(),
            ] }
    }

    fn binding(&self, scope: &AccountScope, session: &Session)
        -> Result<BrowserControlBinding, &'static str> {
        if self.account != scope.account() || &self.user != scope.active_user()
            || self.user != session.active_user {
            return Err(ERROR);
        }
        crate::authorization::CurrentRequestCookies::from_saved_device(session, &self.device)
            .map_err(|_| ERROR)?;
        let cookie = session.cookie_header.split(';')
            .find_map(|part| part.trim().strip_prefix("session=")).ok_or(ERROR)?;
        Ok(BrowserControlBinding { account: scope.account(), user: scope.active_user().clone(),
            session: cookie.to_owned(), device: self.device.clone(), page_url: self.page_url.clone(),
            wrapper_url: self.wrapper_url.clone(), post_path: self.post_path.clone(),
            proxy: scope.browser_control_proxy().map_err(|_| ERROR)?,
        })
    }

    fn validate(&self) -> Result<(), &'static str> {
        let bounded = [&self.user.0, &self.device, &self.page_url, &self.wrapper_url, &self.post_path];
        let path_ok = |p: &Path| p.is_absolute() && p.as_os_str().len() <= 4096
            && !p.as_os_str().as_encoded_bytes().contains(&0);
        let url_ok = |raw: &str, page: bool| reqwest::Url::parse(raw).is_ok_and(|url|
            url.scheme() == "https" && url.host_str().is_some()
            && (!page || url.host_str() == Some("framer.com")) && url.username().is_empty()
            && url.password().is_none() && url.fragment().is_none() && url.as_str() == raw);
        if self.version != 1 || self.account.0.is_nil()
            || bounded.iter().any(|s| s.is_empty() || s.len() > 16 * 1024)
            || !crate::account::device::valid_device_id(&self.device)
            || !url_ok(&self.page_url, true) || !url_ok(&self.wrapper_url, false)
            || !self.post_path.starts_with('/') || self.post_path.bytes().any(|b|
                !(33..=126).contains(&b) || b"?#\\".contains(&b))
            || ![&self.python, &self.script, &self.state_directory].iter().all(|p| path_ok(p))
            || !valid_hash(&self.python_sha256) || !valid_hash(&self.script_sha256)
            || !(1..=180_000).contains(&self.prepare_ms) || !(1..=30_000).contains(&self.probe_ms)
            || !(1..=180_000).contains(&self.witness_ms) {
            return Err(ERROR);
        }
        egress::Endpoint::new(&self.egress_endpoint).map_err(|_| ERROR)?;
        Ok(())
    }
}
