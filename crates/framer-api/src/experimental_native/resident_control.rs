//! Private daemon handoff, not an HTTP endpoint or a settlement receipt.
//! The trusted caller retains its resident lease and all RPC/native completion
//! ownership. These types check context only, never host or native settlement.
use super::*;
use framer_core::{ids::{AccountId, ProjectId}, port::CastleToken};
use std::time::Instant;
use uuid::Uuid;

const CREDENTIAL_LIMIT: usize = 16 * 1024;

/// Identity copied from the retained daemon lease, never from request input.
#[derive(PartialEq, Eq)]
pub struct ResidentLeaseIdentity {
    account: AccountId,
    application_generation: u64,
    host_generation: String,
    account_incarnation: u64,
    lease: Uuid,
}

impl ResidentLeaseIdentity {
    pub fn new(account: AccountId, application_generation: u64,
        host_generation: String, account_incarnation: u64, lease: Uuid)
        -> Result<Self, PreparationFailure> {
        if account.0.is_nil() || application_generation == 0 || lease.is_nil()
            || account_incarnation == 0 || !bounded_text(&host_generation, 128) {
            return Err(PreparationFailure::Rejected);
        }
        Ok(Self { account, application_generation, host_generation,
            account_incarnation, lease })
    }
}

enum LaunchEgress { Direct, Proxy(String) }

impl LaunchEgress {
    fn as_native(&self) -> NativeEgress<'_> {
        match self { Self::Direct => NativeEgress::Direct,
            Self::Proxy(url) => NativeEgress::Proxy(url) }
    }

    fn matches(&self, supplied: &NativeEgress<'_>) -> bool {
        match (self, supplied) {
            (Self::Direct, NativeEgress::Direct) => true,
            (Self::Proxy(expected), NativeEgress::Proxy(actual)) => expected.as_str() == *actual,
            _ => false,
        }
    }
}

/// Trusted owner assertions, not evidence that the lease is still held.
/// Record `acquire_started_at` BEFORE sending acquire, not on IPC receipt.
/// Do not reconstruct this non-Clone context to replay a grant or mint.
pub struct TrustedResidentContext {
    scope: AccountScope,
    identity: ResidentLeaseIdentity,
    target: CredentialTarget,
    page_url: String,
    device_id: String,
    launch_egress: LaunchEgress,
    acquire_started_at: Instant,
}

impl TrustedResidentContext {
    /// The owner supplies its captured launch route and exact resident page.
    /// Neither a configured proxy fallback nor a target editor URL is suitable.
    #[allow(clippy::too_many_arguments)]
    pub fn new(scope: &AccountScope, identity: ResidentLeaseIdentity,
        target: CredentialTarget, page_url: String, device_id: String,
        captured_launch_egress: NativeEgress<'_>, acquire_started_at: Instant)
        -> Result<Self, PreparationFailure> {
        if identity.account != scope.account() || acquire_started_at > Instant::now()
            || !valid_page(&page_url) || !valid_device(&device_id)
            || target.project.0.is_empty() || target.project.0.len() > 256
            || !target.project.0.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || !crate::routes::inference().any(|route| route.path == target.path)
            || !scope.matches_egress(&captured_launch_egress) {
            return Err(PreparationFailure::Rejected);
        }
        let launch_egress = match captured_launch_egress {
            NativeEgress::Direct => LaunchEgress::Direct,
            NativeEgress::Proxy(url) if bounded_text(url, CREDENTIAL_LIMIT) =>
                LaunchEgress::Proxy(url.to_owned()),
            NativeEgress::Proxy(_) => return Err(PreparationFailure::Rejected),
        };
        Ok(Self { scope: scope.clone(), identity, target, page_url, device_id,
            launch_egress, acquire_started_at })
    }
}

fn valid_device(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256
        && value.bytes().all(|b| (0x21..=0x7e).contains(&b) && !b";,\\\"".contains(&b))
}

/// Raw private reply fields. Construction grants no authority or settlement.
/// `cookie_header` must be the actual API-selected current jar; this layer
/// cannot derive browser cookie domain/path selection from a header string.
pub struct ResidentObservation<'a> {
    identity: &'a ResidentLeaseIdentity,
    project: &'a ProjectId,
    path: &'a str,
    page_url: &'a str,
    device_id: &'a str,
    cookie_header: &'a str,
    egress: NativeEgress<'a>,
}

impl<'a> ResidentObservation<'a> {
    pub fn from_unverified_reply(identity: &'a ResidentLeaseIdentity, project: &'a ProjectId,
        path: &'a str, page_url: &'a str, device_id: &'a str, cookie_header: &'a str,
        egress: NativeEgress<'a>) -> Self {
        Self { identity, project, path, page_url, device_id, cookie_header, egress }
    }
}

/// A checked current jar, consumed once by bearer acquisition. Not a lease.
pub struct ResidentGrant {
    context: TrustedResidentContext,
    session: Session,
    current_header: String,
}

/// One validated bearer shared by the browser baseline and native arm.
/// Borrow `as_str()` for the baseline, then consume this value for native use.
pub struct ResidentBearer {
    grant: ResidentGrant,
    value: String,
}

/// One native mint begun after bearer acquisition and the browser baseline.
/// The owner creates this before sending its mint RPC and keeps the same lease.
/// It cannot be cloned or restamped after an IPC result arrives.
pub struct ResidentMint {
    bearer: ResidentBearer,
    started_at: Instant,
}

impl ResidentBearer {
    pub fn as_str(&self) -> &str { &self.value }
}

impl TrustedResidentContext {
    fn check(&self, api: &BoundControlClient, session: &Session,
        actual: &ResidentObservation<'_>) -> Result<CurrentRequestCookies, PreparationFailure> {
        if !api.expected.same_generation(&self.scope)
            || self.scope.active_user() != &session.active_user
            || !self.scope.matches_egress(&self.launch_egress.as_native())
            || actual.identity != &self.identity || actual.project != &self.target.project
            || actual.path != self.target.path || actual.page_url != self.page_url
            || actual.device_id != self.device_id || !self.launch_egress.matches(&actual.egress)
            || actual.cookie_header.len() > CREDENTIAL_LIMIT {
            return Err(PreparationFailure::Rejected);
        }
        let cookies = CurrentRequestCookies::from_current_jar(session, actual.cookie_header.into())
            .map_err(|_| PreparationFailure::Rejected)?;
        let mut device = None;
        for part in actual.cookie_header.split(';') {
            let (name, value) = part.trim_start_matches(' ').split_once('=')
                .ok_or(PreparationFailure::Rejected)?;
            if name == "__cuid" && device.replace(value).is_some() {
                return Err(PreparationFailure::Rejected);
            }
        }
        if device != Some(self.device_id.as_str()) { return Err(PreparationFailure::Rejected); }
        Ok(cookies)
    }
}

impl ResidentGrant {
    pub fn new_checked(api: &BoundControlClient, context: TrustedResidentContext,
        session: &Session, actual: ResidentObservation<'_>) -> Result<Self, PreparationFailure> {
        context.check(api, session, &actual)?;
        Ok(Self { context, session: session.clone(), current_header: actual.cookie_header.into() })
    }
}

impl BoundControlClient {
    pub fn begin_resident_mint(&self, retained: ResidentBearer, session: &Session,
        actual: ResidentObservation<'_>, control: &PreparationControl)
        -> Result<ResidentMint, PreparationFailure> {
        if control.is_cancelled() { return Err(PreparationFailure::Cancelled); }
        if session.active_user != retained.grant.session.active_user
            || session.cookie_header != retained.grant.session.cookie_header
            || actual.cookie_header != retained.grant.current_header
            || retained.value.is_empty() || retained.value.len() > CREDENTIAL_LIMIT {
            return Err(PreparationFailure::Rejected);
        }
        retained.grant.context.check(self, session, &actual)?;
        Ok(ResidentMint { bearer: retained, started_at: Instant::now() })
    }

    /// Consume the mint ticket after rechecking the post-mint observation.
    /// No auth request occurs, and time spent in the mint RPC remains counted.
    pub fn authorize_resident_mint(&self, mint: ResidentMint, session: &Session,
        actual: ResidentObservation<'_>, token: CastleToken, control: &PreparationControl)
        -> Result<PreparedCredentials, PreparationFailure> {
        let ResidentMint { mut bearer, started_at } = mint;
        bearer.grant.context.acquire_started_at = started_at;
        self.authorize_resident_token(bearer, session, actual, token, control)
    }

    /// Recheck the post-mint observation against the retained lease context
    /// and exact bearer jar. Synchronous and consumed once: no second auth call.
    /// This returns credentials only. It never permits release of a host fence.
    pub fn authorize_resident_token(&self, retained: ResidentBearer, session: &Session,
        actual: ResidentObservation<'_>, token: CastleToken, control: &PreparationControl)
        -> Result<PreparedCredentials, PreparationFailure> {
        self.resident_token(retained, session, actual, token, control, false)
    }

    /// Supply joined SDK output under the existing resident jar and bearer.
    /// The timestamp must be the worker's pre-spawn origin, never receipt time.
    pub fn authorize_resident_sdk_mint(&self, mint: ResidentMint, session: &Session,
        actual: ResidentObservation<'_>, token: CastleToken, origin: Instant,
        control: &PreparationControl) -> Result<PreparedCredentials, PreparationFailure> {
        if origin < mint.started_at || origin > Instant::now() {
            return Err(PreparationFailure::Rejected);
        }
        let mut bearer = mint.bearer;
        bearer.grant.context.acquire_started_at = origin;
        self.resident_token(bearer, session, actual, token, control, true)
    }

    fn resident_token(&self, retained: ResidentBearer, session: &Session,
        actual: ResidentObservation<'_>, token: CastleToken, control: &PreparationControl,
        sdk: bool) -> Result<PreparedCredentials, PreparationFailure> {
        if control.is_cancelled() { return Err(PreparationFailure::Cancelled); }
        let ResidentBearer { grant, value } = retained;
        if session.active_user != grant.session.active_user
            || session.cookie_header != grant.session.cookie_header
            || actual.cookie_header != grant.current_header
            || value.is_empty() || value.len() > CREDENTIAL_LIMIT
            || token.0.is_empty() || token.0.len() > CREDENTIAL_LIMIT {
            return Err(PreparationFailure::Rejected);
        }
        let cookies = grant.context.check(self, session, &actual)?;
        let constructor = if sdk { FreshCastleToken::from_sdk_output_at }
            else { FreshCastleToken::from_browser_at };
        let castle = constructor(grant.context.scope.account(), token,
            grant.context.acquire_started_at).map_err(|_| PreparationFailure::Rejected)?;
        Ok(PreparedCredentials { material: CredentialMaterial { bearer: value, castle },
            current_cookies: Some(cookies) })
    }

    /// Consume the checked jar to request exactly one bearer using normal
    /// account-bound auth. The caller must retain this result across both arms.
    /// Preparation cleanup here covers auth HTTP only, not the resident lease.
    pub async fn resident_bearer(&self, grant: ResidentGrant, control: &PreparationControl)
        -> PreparationResult<ResidentBearer> {
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        if !self.expected.same_generation(&grant.context.scope)
            || !grant.context.scope.matches_egress(&grant.context.launch_egress.as_native()) {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        let current = Session { active_user: grant.session.active_user.clone(),
            cookie_header: grant.current_header.clone() };
        let value = match self.bearer(&grant.context.scope, &current).await {
            Ok(value) => value,
            Err(_) => return PreparationResult::unconfirmed(),
        };
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        if !grant.context.scope.matches_egress(&grant.context.launch_egress.as_native()) {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        PreparationResult::finished(Ok(ResidentBearer { grant, value }))
    }
}

fn valid_page(page: &str) -> bool {
    if !bounded_text(page, 4096) || page.chars().any(char::is_whitespace) { return false; }
    let Ok(url) = reqwest::Url::parse(page) else { return false; };
    url.scheme() == "https" && url.host_str() == Some("framer.com")
        && url.username().is_empty() && url.password().is_none() && url.port().is_none()
}

fn bounded_text(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

#[cfg(test)]
#[path = "resident_control_tests.rs"]
mod tests;
