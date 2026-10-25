//! One-shot admission using the serving application's existing owners.
use crate::AppState;
use framer_core::{account::ManagedAccount, convert,
    admission::{Identity, PoolAdmission},
    resident_pool::{Acquire, ResidentLease, ResidentStatus}};
use std::time::Duration;
use tokio::{sync::OwnedSemaphorePermit, time::Instant};

#[must_use = "transfer the original owners into the caller and retain cleanup"]
pub struct AdmittedControl {
    pub identity: Identity,
    pub proxy: Option<String>,
    pub capacity: OwnedSemaphorePermit,
    pub pool: PoolAdmission,
    pub completion: ControlOwner,
}

pub struct ControlOwner {
    operation: Option<framer_core::operation_gate::OperationOwner>,
    resident: Option<ResidentLease>,
    started: bool,
    // Reuse account-management exclusion through resident cleanup. Ordinary
    // admission must refuse locally before probing the reserved browser.
    _reservation: Option<crate::pool_control::Management>,
}
impl ControlOwner {
    pub fn start(&mut self) { self.started = true; }
    pub fn complete(mut self) -> Result<(), AdmissionError> {
        self.operation.take().expect("owned control").complete()
            .map_err(|_| error(AdmissionError::Cleanup))?;
        self.started = false;
        self.resident.take();
        Ok(())
    }
}
impl Drop for ControlOwner {
    fn drop(&mut self) {
        if self.started {
            if let Some(resident) = self.resident.take() { resident.retain_unconfirmed(); }
        } else {
            if let Some(owner) = self.operation.take() { let _ = owner.complete(); }
        }
    }
}

/// Retain an already admitted account through app-owned session cleanup.
/// No browser work, account selection, or second capacity acquisition occurs.
pub fn retain_prepared(s: &AppState, identity: &Identity, pool: &PoolAdmission)
    -> Result<ControlOwner, AdmissionError> {
    if pool.account() != identity.account {
        return Err(error(AdmissionError::InvalidScope));
    }
    let ownership = s.operation_ownership.as_ref()
        .ok_or_else(|| error(AdmissionError::Ineligible))?;
    let mut completion = ControlOwner {
        operation: Some(ownership.external_cleanup()
            .map_err(|_| error(AdmissionError::Ineligible))?),
        resident: None, started: false, _reservation: None,
    };
    require_resident(s, identity.account)?;
    completion.resident = Some(match s.serving_accounts.residents
        .try_acquire(identity.account, None).map_err(|_| error(AdmissionError::Ineligible))? {
        Acquire::Lease(lease) => lease,
        Acquire::Evict(_) => return Err(error(AdmissionError::Internal)),
    });
    Ok(completion)
}

fn error(kind: AdmissionError) -> AdmissionError { kind }

#[derive(Clone, Copy)]
pub enum HealthOutcome { Complete, RiskRefused, Unclassified }

impl ControlOwner {
    /// Re-read while retaining account ownership. A control never clears an
    /// existing cooldown or wave pause, and unknown auth errors stay unknown.
    pub async fn publish(&self, s: &AppState, identity: &Identity,
        proxy: &Option<String>, outcome: HealthOutcome) -> Result<(), AdmissionError> {
        if !self.started
            || self.resident.as_ref().map(ResidentLease::account) != Some(identity.account) {
            return Err(error(AdmissionError::InvalidScope));
        }
        let _update = s.serving_accounts.health_update.lock().await;
        let mut account = s.state.get(identity.account).await
            .map_err(|_| error(AdmissionError::Internal))?
            .ok_or_else(|| error(AdmissionError::Ineligible))?;
        if account.framer_user_id != identity.user || &account.proxy != proxy
            || account.current_team.as_ref() != Some(&identity.team)
            || account.current_project.as_ref() != Some(&identity.project) {
            return Err(error(AdmissionError::InvalidScope));
        }
        if matches!(outcome, HealthOutcome::RiskRefused) {
            let previous = account.cooling_until_ms.unwrap_or(0);
            account.note_risk_refused(framer_core::rotation::now_ms());
            account.cooling_until_ms = Some(previous.max(account.cooling_until_ms.unwrap_or(0)));
            s.wave.note_refusal(&identity.account.0.to_string());
            s.state.upsert(&account).await.map_err(|_| error(AdmissionError::Internal))?;
        }
        Ok(())
    }

    pub async fn revalidate(&self, s: &AppState, identity: &Identity, proxy: &Option<String>)
        -> Result<(), AdmissionError> {
        if self.resident.as_ref().map(ResidentLease::account) != Some(identity.account) {
            return Err(error(AdmissionError::InvalidScope));
        }
        let account = current(s, identity).await?;
        if &account.proxy != proxy { return Err(error(AdmissionError::InvalidScope)); }
        require_resident(s, identity.account)
    }
}

fn require_resident(s: &AppState, account: framer_core::ids::AccountId)
    -> Result<(), AdmissionError> {
    let snapshot = s.serving_accounts.residents.snapshot()
        .map_err(|_| error(AdmissionError::Ineligible))?;
    if !snapshot.residents.iter().any(|slot| slot.account == account
        && slot.users > 0 && !slot.uncertain && !slot.close_started
        && slot.status == ResidentStatus::Active) {
        return Err(error(AdmissionError::Ineligible));
    }
    Ok(())
}

async fn current(s: &AppState, identity: &Identity)
    -> Result<ManagedAccount, AdmissionError> {
    if s.wave.check().is_some() { return Err(error(AdmissionError::Overloaded)); }
    let account = s.state.get(identity.account).await
        .map_err(|_| error(AdmissionError::Internal))?
        .ok_or_else(|| error(AdmissionError::Ineligible))?;
    if !s.serving_accounts.allows(&account.id)
        || !account.is_available_at(framer_core::rotation::now_ms()) {
        return Err(error(AdmissionError::Ineligible));
    }
    if account.framer_user_id != identity.user
        || account.current_team.as_ref() != Some(&identity.team)
        || account.current_project.as_ref() != Some(&identity.project) {
        return Err(error(AdmissionError::InvalidScope));
    }
    Ok(account)
}

/// No account repick, new coordinator, session read, or credential issuance.
pub async fn admit(s: &AppState, identity: Identity, model: &str, deadline: Instant)
    -> Result<AdmittedControl, AdmissionError> {
    admit_with_policy(s, identity, model, deadline, false).await
}

/// Resident controls must not allocate a browser slot or select an eviction.
/// The eventual host lease independently validates the actual live objects.
pub async fn admit_existing(s: &AppState, identity: Identity, model: &str, deadline: Instant)
    -> Result<AdmittedControl, AdmissionError> {
    admit_with_policy(s, identity, model, deadline, true).await
}

async fn admit_with_policy(s: &AppState, identity: Identity, model: &str,
    deadline: Instant, existing_only: bool) -> Result<AdmittedControl, AdmissionError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() || remaining > Duration::from_secs(60)
        || model.is_empty() || model.len() > 256 || model.chars().any(char::is_control)
        || identity.chat_session.is_empty() || identity.chat_session.len() > 256
        || identity.chat_session.chars().any(char::is_control) {
        return Err(error(AdmissionError::InvalidRequest));
    }
    let ownership = s.operation_ownership.clone()
        .ok_or_else(|| error(AdmissionError::Ineligible))?;
    let state = s.clone();
    let model = model.to_owned();
    crate::operation_ownership::run(Some(&ownership),
        framer_core::operation_gate::OperationKind::Inference,
        async move { admit_registered(&state, identity, &model, deadline, existing_only).await })
        .await.map_err(|_| error(AdmissionError::Ineligible))?
}

async fn retain_existing_slot(s: &AppState, account: framer_core::ids::AccountId)
    -> Result<(ResidentLease, crate::pool_control::Management), AdmissionError> {
    let reservation = crate::pool_control::management(s, account, false).await
        .map_err(|_| error(AdmissionError::Ineligible))?;
    let _operation = s.serving_accounts.operation.lock().await;
    if s.serving_accounts.pending_config.read()
            .map_err(|_| error(AdmissionError::Ineligible))?.is_some()
        || !s.serving_accounts.allows(&account) {
        return Err(error(AdmissionError::Ineligible));
    }
    let resident = s.serving_accounts.residents.try_acquire_idle_existing(account)
        .map_err(|_| error(AdmissionError::Ineligible))?;
    Ok((resident, reservation))
}

async fn admit_registered(s: &AppState, identity: Identity, model: &str,
    deadline: Instant, existing_only: bool) -> Result<AdmittedControl, AdmissionError> {
    let ownership = s.operation_ownership.as_ref()
        .ok_or_else(|| error(AdmissionError::Ineligible))?;
    tokio::time::timeout_at(deadline, async {
        let account = current(s, &identity).await?;
        let (resident, reservation) = if existing_only {
            let (resident, reservation) = retain_existing_slot(s, identity.account).await?;
            (resident, Some(reservation))
        } else {
            (crate::lease_account(s, identity.account).await
                .map_err(|_| error(AdmissionError::Ineligible))?, None)
        };
        let operation = ownership.external_cleanup()
            .map_err(|_| error(AdmissionError::Ineligible))?;
        let pool = PoolAdmission::new(resident, Some(operation));
        let mut completion = ControlOwner {
            operation: Some(ownership.external_cleanup()
                .map_err(|_| error(AdmissionError::Ineligible))?),
            resident: None, started: false, _reservation: reservation,
        };
        let key = format!("{}:{}", identity.team.0, convert::to_openrouter_id(model));
        let capacity = s.gate.acquire(&key).await;
        let fresh = current(s, &identity).await?;
        if fresh.proxy != account.proxy { return Err(error(AdmissionError::InvalidScope)); }
        require_resident(s, identity.account)?;
        // The original lease remains occupied, so this cannot select a victim.
        completion.resident = Some(match s.serving_accounts.residents
            .try_acquire(identity.account, None).map_err(|_| error(AdmissionError::Ineligible))? {
            Acquire::Lease(lease) => lease,
            Acquire::Evict(_) => return Err(error(AdmissionError::Internal)),
        });
        Ok(AdmittedControl { identity, proxy: fresh.proxy, capacity, pool, completion })
    }).await.map_err(|_| error(AdmissionError::Deadline))?
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError { InvalidRequest, InvalidScope, Ineligible, Overloaded, Deadline, Cleanup, Internal }
