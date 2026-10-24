use super::*;
use framer_core::port::BrowserAuthorizationRequest;

#[cfg(test)]
#[path = "contextual_control_tests.rs"]
mod tests;

impl BoundBrowserSessionSource {
    pub(super) async fn issue_contextual(&self, scope: &AccountScope, session: &Session,
        target: &CredentialTarget, control: &PreparationControl)
        -> PreparationResult<PreparedCredentials> {
        if !self.api.expected.same_generation(scope) || scope.active_user() != &session.active_user
            || !target.valid_for_browser() || !CurrentRequestCookies::valid_session(session) {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        let account = scope.account();
        let request = BrowserAuthorizationRequest { project: &target.project, path: target.path,
            cookie_header: &session.cookie_header };
        let supplied = match self.browser.prepare_inference_authorization(&account, request).await {
            Ok(value) => value,
            Err(_) => return PreparationResult::unconfirmed(),
        };
        if supplied.cookie_header.len() > 16 * 1024 {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        let current_header = supplied.cookie_header.clone();
        let (castle, cookies) = match self.check_contextual(account, session,
            target, control, supplied) {
            Ok(value) => value,
            Err(error) => return PreparationResult::finished(Err(error)),
        };
        // The browser's checked jar supplies both the bearer request and the
        // native request context. The stored login remains unchanged.
        let current = Session { active_user: session.active_user.clone(),
            cookie_header: current_header };
        let bearer = match self.api.bearer(scope, &current).await {
            Ok(value) => value,
            Err(_) => return PreparationResult::unconfirmed(),
        };
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        PreparationResult::finished(Ok(PreparedCredentials {
            material: CredentialMaterial { bearer, castle }, current_cookies: Some(cookies),
        }))
    }

    fn check_contextual(&self, account: framer_core::ids::AccountId, session: &Session,
        target: &CredentialTarget, control: &PreparationControl,
        supplied: framer_core::port::BrowserAuthorization)
        -> Result<(FreshCastleToken, CurrentRequestCookies), PreparationFailure> {
        if control.is_cancelled() {
            return Err(PreparationFailure::Cancelled);
        }
        if supplied.project != target.project || supplied.path != target.path
            || !target.matches_page(&supplied.page_url)
            || supplied.castle.0.is_empty() || supplied.castle.0.len() > 16 * 1024 {
            return Err(PreparationFailure::Rejected);
        }
        let cookies = match CurrentRequestCookies::from_current_jar(session, supplied.cookie_header) {
            Ok(cookies) => cookies,
            Err(_) => return Err(PreparationFailure::Rejected),
        };
        // Start token age before bearer acquisition. A slow bearer request
        // must not make an old browser token appear freshly received.
        Ok((
            FreshCastleToken::from_browser(account, supplied.castle), cookies,
        ))
    }
}
