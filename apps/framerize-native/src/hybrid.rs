use framer_native::{ServeConfig, transport::browser::BrowserHost};
use std::{future::Future, path::PathBuf, sync::Arc};

pub async fn run(config: ServeConfig, script: PathBuf,
    control: Option<Arc<dyn framer_native::authorization::control::BrowserCredentialControl>>,
    stop: impl Future<Output = ()> + Send + 'static) -> Result<(), String> {
    let script = script.canonicalize().map_err(|_| "browser transport script unavailable")?;
    let browser = Arc::new(BrowserHost::new(&script, &config.profiles.join(".hybrid-transport"))?);
    let result = match control {
        Some(control) => framer_native::serve_with_browser_control(config, control,
            Some(browser.clone()), stop).await,
        None => framer_native::serve_with_browser(config, browser.clone(), stop).await,
    };
    if browser.shutdown().await.is_err() {
        return Err("browser transport shutdown unconfirmed".into());
    }
    result
}
