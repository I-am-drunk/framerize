use super::*;

#[derive(Default)]
pub(super) struct CdpAccountRoute {
    // Remains set if startup is interrupted or cleanup is unconfirmed.
    pub may_have_browser: bool,
    pub group: Option<Arc<cdp_process_group::ProcessGroup>>,
    // Survives an instance dropped during failed or cancelled startup.
    pub pending: Arc<cdp_pending_commands::PendingCommands>,
}

fn has_embedded_credentials(proxy: &str) -> bool {
    proxy.split([';', ',']).any(|server| {
        let server = server.split_once("://").map_or(server, |(_, address)| address);
        let authority = server.split(['/', '?', '#']).next().unwrap_or_default();
        authority.contains('@')
    })
}

impl CdpBridge {
    pub(super) async fn route_cell(&self, account: &AccountId) -> Arc<Mutex<CdpAccountRoute>> {
        self.routes.lock().await.entry(*account).or_default().clone()
    }

    pub(super) async fn register_proxy(&self, account: &AccountId, proxy: Option<String>)
        -> Result<(), String> {
        if proxy.as_ref().map_or(false, String::is_empty) {
            return Err("proxy must be a nonempty string or null".into());
        }
        // Chrome's manual proxy settings do not apply embedded credentials.
        if matches!(self.engine, EngineKind::FullChrome | EngineKind::HeadlessShell)
            && proxy.as_deref().is_some_and(has_embedded_credentials)
        {
            return Err("CDP browser proxy credentials are unsupported".into());
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let cell = self.route_cell(account).await;
            let route = cell.lock().await;
            let instances = self.instances.lock().await;
            let mut proxies = self.proxies.write().await;
            let key = account.0.to_string();
            if proxies.get(&key) == proxy.as_ref() { return Ok(()); }
            if route.may_have_browser || instances.contains_key(account) {
                return Err("account proxy change requires confirmed browser closure".into());
            }
            if self.engine == EngineKind::Attach && proxy.is_some() {
                return Err("an attached browser's proxy cannot be configured by this bridge".into());
            }
            match proxy { Some(value) => { proxies.insert(key, value); }, None => { proxies.remove(&key); } }
            Ok(())
        }).await.map_err(|_| "account proxy registration timed out".to_owned())?
    }
}
