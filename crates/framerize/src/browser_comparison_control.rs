use super::*;
use framer_api::experimental_native::{authorization::{CredentialTarget, PreparedCredentials},
    native_account::{PreparationControl, PreparationResult}};
use framer_core::port::{BrowserAuthorization, Session};

/// Internal port; production always selects the account-bound control client.
pub(crate) trait Control: Send + Sync {
    fn bearer<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        target: &'a CredentialTarget, page: &'a str, cookies: &'a str,
        cancel: &'a PreparationControl) -> FutureResult<'a, PreparationResult<String>>;
    fn authorize<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        target: &'a CredentialTarget, cancel: &'a PreparationControl,
        supplied: BrowserAuthorization) -> FutureResult<'a, PreparationResult<PreparedCredentials>>;
}

pub(crate) trait ControlFactory: Send + Sync {
    fn build(&self, scope: AccountScope, proxy: Option<&str>)
        -> Result<Arc<dyn Control>, Failure>;
}

pub(super) struct BoundFactory;
impl ControlFactory for BoundFactory {
    fn build(&self, scope: AccountScope, proxy: Option<&str>)
        -> Result<Arc<dyn Control>, Failure> {
        let egress = proxy.map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct);
        BoundControlClient::new(scope, egress).map(|client| Arc::new(client) as Arc<dyn Control>)
            .map_err(|_| Failure::Source)
    }
}

impl Control for BoundControlClient {
    fn bearer<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        target: &'a CredentialTarget, page: &'a str, cookies: &'a str,
        cancel: &'a PreparationControl) -> FutureResult<'a, PreparationResult<String>> {
        Box::pin(self.browser_context_bearer(scope, session, target, page, cookies, cancel))
    }
    fn authorize<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        target: &'a CredentialTarget, cancel: &'a PreparationControl,
        supplied: BrowserAuthorization) -> FutureResult<'a, PreparationResult<PreparedCredentials>> {
        Box::pin(self.authorize_browser_token(scope, session, target, cancel, supplied))
    }
}
