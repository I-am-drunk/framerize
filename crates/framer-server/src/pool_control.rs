use super::{ApiError, AppState, PoolConfig, ServingAccountPolicy, POOL_SETTING_KEY};
use super::pool_config::EligibleAccounts;
use axum::{extract::State, Json};
use framer_core::{AccountId, BrowserBridge};
use framer_core::resident_pool::{Acquire, ParkTicket, PoolError, ResidentLease, ResidentStatus};
use serde::Deserialize;
use std::time::Duration;

fn busy(error: impl std::fmt::Display) -> ApiError {
    ApiError::PoolBusy(error.to_string())
}

fn close_failure(expected: &framer_core::browser_lifecycle::BrowserAccountStatus,
    result: &framer_core::browser_lifecycle::IdleCloseResult) -> String {
    if !result.status.same_identity(expected) {
        return "the browser identity changed during closure; the occupied slot is retained".into();
    }
    format!("{}; browser close returned {:?} and its occupied slot is retained",
        result.status.cleanup_explanation(), result.outcome)
}

fn trace_close_refusal(account: AccountId,
    expected: &framer_core::browser_lifecycle::BrowserAccountStatus,
    result: &framer_core::browser_lifecycle::IdleCloseResult) {
    super::operation_ownership::correlate_chain(
        framer_core::observability::Event::new("pool.close.refused"))
        .account(&account)
        .field("outcome", format!("{:?}", result.outcome))
        .field("state", format!("{:?}", result.status.state))
        .field("identity_matches", result.status.same_identity(expected))
        .field("active_commands", result.status.active_commands)
        .field("close_not_started", result.confirms_busy(expected))
        .field("cause", close_failure(expected, result))
        .emit();
}

// A completed call may still own an unresolved close or observation.
// Preserve that evidence separately from the error reported to the caller.
pub(super) struct CleanupOutcome<T> {
    pub(super) result: Result<T, ApiError>,
    pub(super) cleanup_confirmed: bool,
}

pub(super) type AcquisitionOutcome<T> = CleanupOutcome<T>;

impl<T: axum::response::IntoResponse> axum::response::IntoResponse for CleanupOutcome<T> {
    fn into_response(self) -> axum::response::Response {
        super::operation_ownership::completed_response(
            axum::response::IntoResponse::into_response(self.result), self.cleanup_confirmed)
    }
}

#[derive(Clone)]
pub(super) struct CleanupEvidence(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl CleanupEvidence {
    pub(super) fn new() -> Self { Self(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true))) }
    pub(super) fn confirmed(&self) -> bool { self.0.load(std::sync::atomic::Ordering::Relaxed) }
    pub(super) fn include(&self, confirmed: bool) {
        if !confirmed { self.0.store(false, std::sync::atomic::Ordering::Relaxed); }
    }
    pub(super) fn outcome<T>(&self, outcome: CleanupOutcome<T>) -> Result<T, ApiError> {
        self.include(outcome.cleanup_confirmed);
        if !outcome.cleanup_confirmed {
            if let Err(ApiError::Internal(error) | ApiError::PoolBusy(error)
                | ApiError::ServingCapacity(error) | ApiError::InvalidRequest(error)) = &outcome.result {
                super::operation_ownership::record_cleanup_failure(error);
            }
        }
        outcome.result
    }
    pub(super) fn external<T>(&self, result: Result<T, String>) -> Result<T, String> {
        if let Err(error) = &result {
            if cleanup_unconfirmed(error) {
                super::operation_ownership::record_cleanup_failure(error);
            }
        }
        self.include(!matches!(&result, Err(error) if cleanup_unconfirmed(error)));
        result
    }
    pub(super) fn core<T>(&self, result: Result<T, framer_core::error::CoreError>)
        -> Result<T, framer_core::error::CoreError> {
        if let Err(framer_core::error::CoreError::Framer(error)) = &result {
            if cleanup_unconfirmed(error) {
                super::operation_ownership::record_cleanup_failure(error);
            }
        }
        self.include(!matches!(&result, Err(framer_core::error::CoreError::Framer(error))
            if cleanup_unconfirmed(error)));
        result
    }
    pub(super) fn finish<T>(&self, result: Result<T, ApiError>) -> CleanupOutcome<T> {
        CleanupOutcome { result, cleanup_confirmed: self.confirmed() }
    }
    pub(super) fn admission<T>(&self, result: Result<T, ApiError>) -> Result<T, ApiError> {
        if let Err(error) = &result {
            self.include(matches!(error, ApiError::Cooling { .. }
                | ApiError::AccountCooling { .. } | ApiError::ServingCapacity(_)));
        }
        result
    }
}

async fn confirm_resident_status(browser: &dyn BrowserBridge, lease: ResidentLease)
    -> CleanupOutcome<ResidentLease> {
    use framer_core::browser_lifecycle::BrowserAccountState as State;
    let mut cleanup_confirmed = true;
    let result = async {
    if !browser.supports_browser_lifecycle() {
        lease.ensure_admissible().map_err(|error| {
            if error != PoolError::AccountBusy { cleanup_confirmed = false; }
            busy(error)
        })?;
        return Ok(lease);
    }
    cleanup_confirmed = false;
    let observation = tokio::time::timeout(Duration::from_secs(5),
        browser.browser_status(&lease.account())).await;
    let failure;
    let usable = match observation {
        Ok(Ok(status)) => {
            let identity_valid = match status.host_generation.as_deref() {
                Some(generation) => !generation.is_empty()
                    && (status.account_incarnation > 0 || status.state == State::Absent),
                None => status.account_incarnation == 0 && status.state == State::Absent,
            };
            // A matching, completed read can refuse account admission without
            // leaving request work unfinished. The resident fence owns the
            // unavailable account; a failed transport still remains unknown.
            cleanup_confirmed = status.account_id == lease.account() && identity_valid;
            failure = if status.account_id != lease.account() {
                "browser status belongs to a different account".into()
            } else if !identity_valid {
                "browser status has a missing or invalid process identity".into()
            } else { status.cleanup_explanation() };
            cleanup_confirmed
                && !matches!(status.state, State::Closing | State::Unconfirmed)
        },
        Ok(Err(error)) => {
            failure = format!("browser status check failed: {}",
                framer_core::error::safe_failure_summary(&error));
            false
        },
        Err(_) => {
            failure = "browser status check did not reply within 5 seconds".into();
            false
        },
    };
    if !usable {
        lease.mark_unconfirmed_with_cause(&failure).map_err(|error| {
            cleanup_confirmed = false;
            busy(error)
        })?;
        if !cleanup_confirmed { super::operation_ownership::record_cleanup_failure(&failure); }
        return Err(busy(format!("{failure}; the occupied browser slot is retained")));
    }
    lease.ensure_admissible().map_err(|error| {
        // A concurrent local fence refuses this lease without starting work.
        if error != PoolError::AccountBusy { cleanup_confirmed = false; }
        busy(error)
    })?;
    Ok(lease)
    }.await;
    CleanupOutcome { result, cleanup_confirmed }
}

pub(super) fn cleanup_unconfirmed(error: &str) -> bool {
    let cause = error.strip_prefix("chunk ")
        .and_then(|rest| rest.split_once(" retry died (fallback off): "))
        .filter(|(index, _)| index.parse::<u32>().is_ok())
        .map(|(_, cause)| cause).unwrap_or(error);
    matches!(cause, "account browser cleanup is unconfirmed; retry after cleanup"
        | "browser launch cleanup is unconfirmed")
        || cause.contains("host-lifecycle-unconfirmed:")
        || cause.contains("sidecar timeout")
        || cause.contains("sidecar account command timed out")
        || cause.contains("sidecar dropped the request")
        || cause.contains("sidecar write:")
        || cause.starts_with("browser cleanup unconfirmed:")
}

pub(super) fn observe_lease_result<T>(lease: &ResidentLease, result: Result<T, String>)
    -> Result<T, String> {
    if let Err(error) = &result {
        if cleanup_unconfirmed(error) {
            super::operation_ownership::record_cleanup_failure(error);
            lease.mark_unconfirmed_with_cause(error)
                .map_err(|e| format!("account residency fence failed: {e}"))?;
        }
    }
    result
}

#[derive(Clone)]
pub(super) struct RequestResidency(std::sync::Arc<std::sync::Mutex<Option<ResidentLease>>>);

impl RequestResidency {
    pub(super) fn empty() -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(None)))
    }
    pub(super) fn new(lease: ResidentLease) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(Some(lease))))
    }
    pub(super) fn take(&self) -> Option<ResidentLease> {
        self.0.lock().expect("request residency poisoned").take()
    }
    pub(super) fn replace(&self, lease: ResidentLease) {
        let previous = self.0.lock().expect("request residency poisoned").replace(lease);
        drop(previous);
    }
    pub(super) fn observe_result<T>(&self, account: AccountId, result: Result<T, String>)
        -> Result<T, String> {
        if !matches!(&result, Err(error) if cleanup_unconfirmed(error)) { return result; }
        let held = self.0.lock().map_err(|_| "request residency is unavailable")?;
        let lease = held.as_ref().filter(|lease| lease.account() == account)
            .ok_or("cleanup observation has no matching account lease")?;
        observe_lease_result(lease, result)
    }
}

pub(super) struct Management {
    account: AccountId,
    mutations: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<AccountId>>>,
    _lease: Option<ResidentLease>,
}

impl Drop for Management {
    fn drop(&mut self) {
        if let Ok(mut mutations) = self.mutations.lock() { mutations.remove(&self.account); }
    }
}

impl Management {
    pub(super) fn observe_result<T>(&self, result: Result<T, String>) -> Result<T, String> {
        match &self._lease {
            Some(lease) => observe_lease_result(lease, result),
            None => result,
        }
    }
}

pub(super) async fn management(s: &AppState, id: AccountId, browser: bool)
    -> Result<Management, ApiError> {
    management_inner(s, id, browser, false).await
}

pub(super) async fn management_observed(s: &AppState, id: AccountId, browser: bool)
    -> CleanupOutcome<Management> {
    management_inner_observed(s, id, browser, false).await
}

async fn management_inner(s: &AppState, id: AccountId, browser: bool, reconcile: bool)
    -> Result<Management, ApiError> {
    management_inner_observed(s, id, browser, reconcile).await.result
}

async fn management_inner_observed(s: &AppState, id: AccountId, browser: bool, reconcile: bool)
    -> CleanupOutcome<Management> {
    let cleanup = CleanupEvidence::new();
    let result = async {
    let lease = if browser {
        Some(cleanup.outcome(s.serving_accounts.acquire_observed(
            s.browser.as_ref(), id, None, None).await)?)
    } else { None };
    let operation = s.serving_accounts.operation.lock().await;
    if s.serving_accounts.pending_config.read().map_err(busy)?.is_some() {
        return Err(busy("pool configuration is in progress"));
    }
    let snapshot = s.serving_accounts.residents.snapshot().map_err(busy)?;
    if let Some(slot) = snapshot.residents.iter().find(|slot| slot.replacement == Some(id) || (slot.account == id
        && (slot.users > usize::from(lease.is_some()) || (slot.uncertain && !reconcile)
            || slot.status == ResidentStatus::Closing))) {
        let cause = if slot.replacement == Some(id) {
            "the account is reserved while another browser finishes closing".into()
        } else if slot.users > usize::from(lease.is_some()) {
            format!("{} other operations still hold this account's browser", slot.users - usize::from(lease.is_some()))
        } else if let Some(cause) = &slot.cleanup_cause {
            cause.clone()
        } else if slot.uncertain {
            "the browser owner did not confirm cleanup or retain its failure cause".into()
        } else { "the account browser is still closing".into() };
        return Err(busy(cause));
    }
    let mutations = s.serving_accounts.mutations.clone();
    if !mutations.lock().map_err(busy)?.insert(id) {
        return Err(busy("account management is already in progress"));
    }
    drop(operation);
    Ok(Management { account: id, mutations, _lease: lease })
    }.await;
    cleanup.finish(result)
}

pub(super) async fn select_account_observed(
    s: &AppState, accounts: Vec<framer_core::account::ManagedAccount>,
    requested: Option<&str>, excluded: &[String],
) -> AcquisitionOutcome<ResidentLease> {
    let preferred = match s.serving_accounts.preferred_idle(&accounts) {
        Ok(preferred) => preferred,
        Err(error) => return AcquisitionOutcome {
            result: Err(error), cleanup_confirmed: true,
        },
    };
    s.serving_accounts.acquire_selected_observed(s.browser.as_ref(), || {
        let selected = if requested.is_some() || excluded.is_empty() {
            s.serving_accounts.select(accounts.clone(), requested)
        } else {
            s.serving_accounts.select_fresh(accounts.clone(), excluded)
        };
        selected.map(|account| account.id)
    }, preferred, Some(s.state.as_ref())).await
}

pub(super) async fn lease_account(s: &AppState, id: AccountId) -> Result<ResidentLease, ApiError> {
    lease_account_observed(s, id).await.result
}

pub(super) async fn lease_account_observed(s: &AppState, id: AccountId) -> AcquisitionOutcome<ResidentLease> {
    let mut cleanup_confirmed = true;
    let result = async {
        let accounts = s.state.list().await.map_err(ApiError::Internal)?;
        if !accounts.iter().any(|account| account.id == id) {
            return Err(ApiError::InvalidRequest("account not found".into()));
        }
        let preferred = s.serving_accounts.preferred_idle(&accounts)?;
        let acquired = s.serving_accounts.acquire_observed(s.browser.as_ref(), id, preferred, Some(s.state.as_ref())).await;
        cleanup_confirmed = acquired.cleanup_confirmed;
        let lease = acquired.result?;
        if s.state.get(id).await.map_err(ApiError::Internal)?.is_none() {
            return Err(ApiError::InvalidRequest("account was removed before admission".into()));
        }
        Ok(lease)
    }.await;
    AcquisitionOutcome { result, cleanup_confirmed }
}

pub(super) async fn park(state: State<AppState>, body: Json<AccountBody>)
    -> Result<Json<serde_json::Value>, ApiError> {
    park_observed(state, body).await.result
}

pub(super) async fn park_observed(State(s): State<AppState>, Json(body): Json<AccountBody>)
    -> CleanupOutcome<Json<serde_json::Value>> {
    let cleanup = CleanupEvidence::new();
    let result = async {
    let _management = cleanup.outcome(management_inner_observed(&s, body.account_id, false, true).await)?;
    let parked = tokio::time::timeout(Duration::from_secs(20),
        s.serving_accounts.park_managed_observed(s.browser.as_ref(), body.account_id)).await;
    match parked {
        Ok(parked) => cleanup.outcome(parked)?,
        Err(_) => {
            cleanup.include(false);
            return Err(busy("account close timed out; its slot is retained"));
        }
    }
    Ok(Json(serde_json::json!({"account_id":body.account_id,"status":"parked",
        "eligible":s.serving_accounts.allows(&body.account_id)})))
    }.await;
    cleanup.finish(result)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfigUpdate {
    active_capacity: Option<usize>,
    eligible_accounts: Option<EligibleAccounts>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AccountBody { account_id: AccountId }

pub(super) async fn config(State(s): State<AppState>) -> Json<PoolConfig> {
    Json(s.serving_accounts.config())
}

pub(super) async fn activate(state: State<AppState>, body: Json<AccountBody>)
    -> Result<Json<serde_json::Value>, ApiError> {
    activate_observed(state, body).await.result
}

pub(super) async fn activate_observed(State(s): State<AppState>, Json(body): Json<AccountBody>)
    -> CleanupOutcome<Json<serde_json::Value>> {
    let cleanup = CleanupEvidence::new();
    let result = async {
    let accounts = s.state.list().await.map_err(ApiError::Internal)?;
    s.serving_accounts.select(accounts, Some(&body.account_id.0.to_string()))?;
    if let Some(remaining) = s.wave.check() {
        return Err(ApiError::Cooling { retry_after_secs: remaining.as_secs().saturating_add(1) });
    }
    let lease = cleanup.outcome(lease_account_observed(&s, body.account_id).await)?;
    cleanup.admission(super::current_account(&s, body.account_id).await)?;
    let result = cleanup.external(s.browser.castle_token(&body.account_id).await);
    observe_lease_result(&lease, result).map_err(ApiError::Internal)?;
    Ok(Json(serde_json::json!({"account_id":body.account_id,"status":"active"})))
    }.await;
    cleanup.finish(result)
}

pub async fn prewarm_accounts(s: AppState) {
    let Ok(mut accounts) = s.state.list().await else { return; };
    let now = framer_core::rotation::now_ms();
    accounts.retain(|account| s.serving_accounts.allows(&account.id) && account.is_available_at(now));
    accounts.sort_by_key(|account| (account.consecutive_failures, account.id.0));
    for account in accounts.into_iter().take(s.serving_accounts.config().active_capacity) {
        if s.wave.check().is_some() { break; }
        let t0 = std::time::Instant::now();
        let result = activate(State(s.clone()), Json(AccountBody { account_id: account.id })).await;
        framer_core::observability::Event::new("account.prewarm").account(&account.id)
            .duration_ms(framer_core::observability::elapsed_ms(t0))
            .field("cause", result.as_ref().err().map(ApiError::diagnostic))
            .outcome(if result.is_ok() { "ok" } else { "error" }).emit();
    }
}

pub(super) async fn configure(state: State<AppState>, body: Json<ConfigUpdate>)
    -> Result<Json<PoolConfig>, ApiError> {
    configure_observed(state, body).await.result
}

pub(super) async fn configure_observed(State(s): State<AppState>, Json(update): Json<ConfigUpdate>)
    -> CleanupOutcome<Json<PoolConfig>> {
    let cleanup = CleanupEvidence::new();
    let result = async {
    let policy = &s.serving_accounts;
    let _configuration = policy.configuration.lock().await;
    let old = policy.config();
    let mut next = old.clone();
    if let Some(capacity) = update.active_capacity { next.active_capacity = capacity; }
    if let Some(eligible) = update.eligible_accounts { next.eligible_accounts = eligible; }
    next.validate().map_err(ApiError::InvalidRequest)
        .map_err(super::operation_ownership::completed_preflight)?;
    let operation = policy.operation.lock().await;
    if !policy.mutations.lock().map_err(busy)?.is_empty() {
        return Err(busy("account management is in progress"));
    }
    // Account removal uses this same transition lock. Validate after taking
    // it so a stale list cannot publish eligibility for a removed account.
    if let EligibleAccounts::Only(ids) = &next.eligible_accounts {
        let accounts = s.state.list().await.map_err(ApiError::Internal)?;
        if ids.iter().any(|id| !accounts.iter().any(|account| account.id == *id)) {
            return Err(ApiError::InvalidRequest("eligible_accounts contains an unknown account".into()));
        }
    }
    let snapshot = policy.residents.snapshot().map_err(busy)?;
    if snapshot.residents.iter().any(|slot| slot.status == ResidentStatus::Closing) {
        return Err(busy("account browser closure is already in progress"));
    }
    let active = snapshot.residents.iter().filter(|slot| slot.users > 0).count();
    if active > next.active_capacity || snapshot.residents.iter()
        .any(|slot| slot.users > 0 && !next.allows(&slot.account)) {
        return Err(busy("requested pool change would interrupt active work; retry after it finishes"));
    }
    let _pending = PendingConfig::new(policy, next.clone())?;
    let tickets = reserve_config(policy, &next)?;
    drop(operation);
    apply_config(&s, next, tickets, &cleanup).await.map(Json)
    }.await;
    cleanup.finish(result)
}

struct PendingConfig<'a>(&'a ServingAccountPolicy);

impl<'a> PendingConfig<'a> {
    fn new(policy: &'a ServingAccountPolicy, next: PoolConfig) -> Result<Self, ApiError> {
        *policy.pending_config.write().map_err(busy)? = Some(next);
        Ok(Self(policy))
    }
}

impl Drop for PendingConfig<'_> {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.0.pending_config.write() { *pending = None; }
    }
}

// Queued tickets have not issued a close. Restore those reservations on any
// earlier failure or cancellation; only an attempted close can be uncertain.
struct QueuedParks(std::collections::VecDeque<ParkTicket>);

impl Drop for QueuedParks {
    fn drop(&mut self) {
        for ticket in self.0.drain(..) { let _ = ticket.cancel_before_io(); }
    }
}

fn reserve_config(policy: &ServingAccountPolicy, next: &PoolConfig) -> Result<QueuedParks, ApiError> {
    let snapshot = policy.residents.snapshot().map_err(busy)?;
    let mut victims: Vec<_> = snapshot.residents.iter().filter(|slot| !next.allows(&slot.account))
        .map(|slot| slot.account).collect();
    for id in policy.residents.idle_ids().map_err(busy)? {
        if snapshot.occupied.saturating_sub(victims.len()) <= next.active_capacity { break; }
        if !victims.contains(&id) { victims.push(id); }
    }
    if snapshot.occupied.saturating_sub(victims.len()) > next.active_capacity {
        let causes: Vec<_> = snapshot.residents.iter()
            .filter(|slot| !victims.contains(&slot.account))
            .map(|slot| slot.cleanup_cause.clone().unwrap_or_else(||
                format!("browser state {:?} with {} active owners", slot.status, slot.users)))
            .collect();
        return Err(busy(format!("Cannot reduce browser capacity to {}: {}",
            next.active_capacity, causes.join("; "))));
    }
    let mut queued = QueuedParks(Default::default());
    for id in victims {
        queued.0.push_back(policy.residents.begin_park(id).map_err(busy)?);
    }
    Ok(queued)
}

async fn apply_config(s: &AppState, next: PoolConfig, mut tickets: QueuedParks,
    cleanup: &CleanupEvidence) -> Result<PoolConfig, ApiError> {
    let policy = &s.serving_accounts;
    while let Some(ticket) = tickets.0.pop_front() {
        cleanup.outcome(finish_park_observed(s.browser.as_ref(), ticket).await)?;
    }
    let json = serde_json::to_string(&next).map_err(|e| ApiError::Internal(e.to_string()))?;
    let _operation = policy.operation.lock().await;
    s.state.save_setting(POOL_SETTING_KEY, &json).await.map_err(ApiError::Internal)?;
    policy.residents.set_capacity(next.active_capacity).map_err(busy)?;
    *policy.config.write().map_err(|_| busy("pool configuration is unavailable"))? = next.clone();
    Ok(next)
}

impl ServingAccountPolicy {
    pub(super) fn preferred_idle(&self, accounts: &[framer_core::account::ManagedAccount])
        -> Result<Option<AccountId>, ApiError> {
        let now = framer_core::rotation::now_ms();
        Ok(self.residents.idle_ids().map_err(busy)?.into_iter().find(|victim|
            accounts.iter().any(|account| account.id == *victim
                && (!self.allows(victim) || !account.is_available_at(now)))))
    }

    /// The caller retains this account's management guard during the close.
    pub(super) async fn park_managed(&self, browser: &dyn BrowserBridge, id: AccountId) -> Result<(), ApiError> {
        self.park_managed_observed(browser, id).await.result
    }

    pub(super) async fn park_managed_observed(&self, browser: &dyn BrowserBridge, id: AccountId)
        -> CleanupOutcome<()> {
        let cleanup = CleanupEvidence::new();
        let result = async {
        let operation = self.operation.lock().await;
        let ticket = match self.residents.begin_park(id) {
            Ok(ticket) => ticket,
            Err(PoolError::NotResident) => return Ok(()),
            Err(error) => return Err(busy(error)),
        };
        drop(operation);
        cleanup.outcome(finish_park_observed(browser, ticket).await)
        }.await;
        cleanup.finish(result)
    }
}

#[cfg(test)]
#[path = "pool_control_tests.rs"]
mod tests;

async fn finish_park_observed(browser: &dyn BrowserBridge, mut ticket: ParkTicket) -> CleanupOutcome<()> {
    let mut cleanup_confirmed = false;
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        let id = ticket.victim();
        let expected = match browser.browser_status(&id).await {
            Ok(status) => status,
            Err(error) => {
                ticket.cancel_before_io().map_err(busy)?;
                cleanup_confirmed = true;
                return Err(busy(error));
            }
        };
        if expected.account_id != id || expected.active_commands != 0 {
            ticket.cancel_before_io().map_err(busy)?;
            cleanup_confirmed = true;
            return Err(busy("account browser is busy or its identity changed"));
        }
        ticket.close_started().map_err(busy)?;
        let result = browser.close_idle_browser(&id, &expected).await.map_err(|error| {
            ticket.record_failure(&error);
            super::operation_ownership::record_cleanup_failure(&error);
            busy(error)
        })?;
        if !result.confirms_absence(&expected) {
            trace_close_refusal(id, &expected, &result);
            if result.confirms_busy(&expected) {
                ticket.confirm_busy(&expected, &result).map_err(busy)?;
                cleanup_confirmed = true;
                return Err(busy("account browser became busy before closure; its slot is retained"));
            }
            let cause = close_failure(&expected, &result);
            ticket.record_failure(&cause);
            super::operation_ownership::record_cleanup_failure(&cause);
            return Err(busy(cause));
        }
        ticket.confirm_close().map_err(busy)?;
        cleanup_confirmed = true;
        Ok(())
    }).await.unwrap_or_else(|_| Err(busy("account close timed out; its slot is retained")));
    CleanupOutcome { result, cleanup_confirmed }
}

impl ServingAccountPolicy {
    pub(super) async fn acquire(&self, browser: &dyn BrowserBridge, id: AccountId,
        preferred_idle: Option<AccountId>, existing: Option<&dyn framer_core::StateStore>) -> Result<ResidentLease, ApiError> {
        self.acquire_observed(browser, id, preferred_idle, existing).await.result
    }

    async fn acquire_observed(&self, browser: &dyn BrowserBridge, id: AccountId,
        preferred_idle: Option<AccountId>, existing: Option<&dyn framer_core::StateStore>) -> AcquisitionOutcome<ResidentLease> {
        self.acquire_selected_observed(browser, || Ok(id), preferred_idle, existing).await
    }

    pub(super) async fn acquire_selected_observed(
        &self, browser: &dyn BrowserBridge,
        mut choose: impl FnMut() -> Result<AccountId, ApiError> + Send,
        preferred_idle: Option<AccountId>, existing: Option<&dyn framer_core::StateStore>,
    ) -> AcquisitionOutcome<ResidentLease> {
        // This survives timeout cancellation. Before reservation there is no
        // new cleanup to prove; a live ticket or observation changes that.
        let mut cleanup_confirmed = true;
        let result = tokio::time::timeout(Duration::from_secs(25), async {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                let operation = self.operation.lock().await;
                // Select and reserve before another admission can inspect load.
                // Browser I/O below happens after this lock is released.
                let id = choose()?;
                self.require(&id)?;
                if let Some(next) = self.pending_config.read().map_err(busy)?.as_ref() {
                    let resident = self.residents.resident_ids().map_err(busy)?.contains(&id);
                    if !next.allows(&id) || !resident {
                        return Err(busy("pool configuration is in progress; retry admission after it finishes"));
                    }
                }
                let mutations = self.mutations.lock().map_err(busy)?.clone();
                if mutations.contains(&id) { return Err(busy("account management is in progress")); }
                // Management cannot start while this transition is held. Recheck
                // existence after waiting, before reserving or evicting any slot.
                if let Some(state) = existing {
                    if state.get(id).await.map_err(ApiError::Internal)?.is_none() {
                        return Err(ApiError::InvalidRequest("account not found".into()));
                    }
                }
                let protected: Vec<_> = mutations.into_iter().collect();
                match self.residents.try_acquire_excluding(id, preferred_idle, &protected) {
                    Ok(Acquire::Lease(lease)) => {
                        cleanup_confirmed = false;
                        drop(operation);
                        let result = confirm_resident_status(browser, lease).await;
                        cleanup_confirmed = result.cleanup_confirmed;
                        return result.result;
                    },
                    Ok(Acquire::Evict(mut ticket)) => {
                        cleanup_confirmed = false;
                        drop(operation);
                        let expected = match browser.browser_status(&ticket.victim()).await {
                            Ok(status) => status,
                            Err(error) => {
                                ticket.cancel_before_io().map_err(busy)?;
                                cleanup_confirmed = true;
                                return Err(busy(error));
                            }
                        };
                        if expected.account_id != ticket.victim() || expected.active_commands != 0 {
                            ticket.cancel_before_io().map_err(busy)?;
                            cleanup_confirmed = true;
                            return Err(busy("account browser is still busy or its identity changed"));
                        }
                        ticket.close_started().map_err(busy)?;
                        let result = browser.close_idle_browser(&ticket.victim(), &expected)
                            .await.map_err(|error| {
                                ticket.record_failure(&error);
                                super::operation_ownership::record_cleanup_failure(&error);
                                busy(error)
                            })?;
                        if !result.confirms_absence(&expected) {
                            trace_close_refusal(ticket.victim(), &expected, &result);
                            if result.confirms_busy(&expected) {
                                ticket.confirm_busy(&expected, &result).map_err(busy)?;
                                cleanup_confirmed = true;
                                return Err(busy("account browser became busy before closure; its slot is retained"));
                            }
                            let cause = close_failure(&expected, &result);
                            ticket.record_failure(&cause);
                            super::operation_ownership::record_cleanup_failure(&cause);
                            return Err(busy(cause));
                        }
                        let lease = ticket.confirm_close().map_err(busy)?;
                        let result = confirm_resident_status(browser, lease).await;
                        cleanup_confirmed = result.cleanup_confirmed;
                        return result.result;
                    }
                    Err(PoolError::NoIdleSlot | PoolError::AccountBusy) if tokio::time::Instant::now() < deadline => {}
                    Err(error) => return Err(busy(error)),
                }
                drop(operation);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }).await.unwrap_or_else(|_| Err(busy("account admission timed out; occupied slots are retained")));
        AcquisitionOutcome { result, cleanup_confirmed }
    }
}
