//! Selection and residency share one policy; saved identity and health are separate.

use framer_core::account::ManagedAccount;
use framer_core::ids::AccountId;
use std::collections::HashSet;
use std::sync::RwLock;
use framer_core::resident_pool::ResidentPool;
use super::pool_config::{EligibleAccounts, PoolConfig};

use super::ApiError;

pub const SERVING_ACCOUNTS_ENV: &str = "FRAMERIZE_SERVING_ACCOUNTS";

#[derive(Debug)]
pub struct ServingAccountPolicy {
    rotation: std::sync::Arc<framer_core::rotation::RotationState>,
    pub(crate) config: RwLock<PoolConfig>,
    pub(crate) residents: ResidentPool,
    pub(crate) operation: tokio::sync::Mutex<()>,
    pub(crate) configuration: tokio::sync::Mutex<()>,
    pub(crate) health_update: tokio::sync::Mutex<()>,
    // Keep duplicate-email validation and initial publication in one interval.
    pub(crate) registration: tokio::sync::Mutex<()>,
    pub(crate) pending_config: RwLock<Option<PoolConfig>>,
    pub(crate) mutations: std::sync::Arc<std::sync::Mutex<HashSet<AccountId>>>,
}

impl Default for ServingAccountPolicy {
    fn default() -> Self { Self::from_config(PoolConfig::default()).expect("default pool") }
}

impl ServingAccountPolicy {
    pub(crate) fn rotation(&self) -> &std::sync::Arc<framer_core::rotation::RotationState> {
        &self.rotation
    }
    pub fn from_config(config: PoolConfig) -> Result<Self, String> {
        config.validate()?;
        let residents = ResidentPool::new(config.active_capacity).map_err(|e| e.to_string())?;
        Ok(Self { config: RwLock::new(config), residents,
            rotation: Default::default(),
            operation: tokio::sync::Mutex::new(()),
            configuration: tokio::sync::Mutex::new(()),
            health_update: tokio::sync::Mutex::new(()),
            registration: tokio::sync::Mutex::new(()),
            pending_config: RwLock::new(None),
            mutations: Default::default() })
    }

    pub fn config(&self) -> PoolConfig {
        self.config.read().expect("pool config poisoned").clone()
    }

    pub fn from_env() -> Result<Self, String> {
        Self::from_config(Self::config_from_env()?)
    }

    pub fn config_from_env() -> Result<PoolConfig, String> {
        match std::env::var(SERVING_ACCOUNTS_ENV) {
            Ok(value) => Self::parse_config(Some(&value)),
            Err(std::env::VarError::NotPresent) => Self::parse_config(None),
            Err(std::env::VarError::NotUnicode(_)) => {
                Err(format!("{SERVING_ACCOUNTS_ENV} must contain UTF-8 UUIDs"))
            }
        }
    }

    /// Parse full account UUIDs separated by commas. Empty or malformed
    /// configuration fails closed instead of accidentally loading all accounts.
    pub fn parse(raw: Option<&str>) -> Result<Self, String> {
        Self::from_config(Self::parse_config(raw)?)
    }

    pub fn parse_config(raw: Option<&str>) -> Result<PoolConfig, String> {
        let Some(raw) = raw else {
            return Ok(PoolConfig::default());
        };
        let mut allowed = HashSet::new();
        for (index, entry) in raw.split(',').enumerate() {
            let id = uuid::Uuid::parse_str(entry.trim()).map(AccountId).map_err(|_| {
                format!("{SERVING_ACCOUNTS_ENV} entry {} must be a full account UUID", index + 1)
            })?;
            if !allowed.insert(id) {
                return Err(format!("{SERVING_ACCOUNTS_ENV} contains a duplicate account UUID"));
            }
        }
        let mut ids: Vec<_> = allowed.into_iter().collect();
        ids.sort_by_key(|id| id.0);
        Ok(PoolConfig { eligible_accounts: EligibleAccounts::Only(ids),
            ..PoolConfig::default() })
    }

    pub fn allows(&self, id: &AccountId) -> bool {
        self.config.read().map(|config| config.allows(id)).unwrap_or(false)
    }

    pub(super) fn residency_allows(snapshot: &framer_core::resident_pool::PoolSnapshot,
        id: &AccountId, protected: &HashSet<AccountId>) -> bool {
        use framer_core::resident_pool::ResidentStatus;
        if protected.contains(id) || snapshot.residents.iter()
            .any(|slot| slot.replacement == Some(*id)) { return false; }
        if let Some(slot) = snapshot.residents.iter().find(|slot| slot.account == *id) {
            return !slot.uncertain
                && matches!(slot.status, ResidentStatus::Active | ResidentStatus::Idle);
        }
        snapshot.occupied < snapshot.capacity || snapshot.residents.iter().any(|slot|
            slot.users == 0 && !slot.uncertain && slot.status == ResidentStatus::Idle
                && !protected.contains(&slot.account))
    }

    fn residency_failure(snapshot: &framer_core::resident_pool::PoolSnapshot,
        id: &AccountId, protected: &HashSet<AccountId>) -> String {
        if protected.contains(id) {
            return "account management is in progress".into();
        }
        if snapshot.residents.iter().any(|slot| slot.replacement == Some(*id)) {
            return "the account is reserved while another browser finishes closing".into();
        }
        if let Some(slot) = snapshot.residents.iter().find(|slot| slot.account == *id) {
            if let Some(cause) = &slot.cleanup_cause { return cause.clone(); }
            if slot.uncertain {
                return "the browser owner did not confirm cleanup or retain its failure cause".into();
            }
            return "the account browser is still closing".into();
        }
        format!("all {} resident browser slots are occupied; none is idle and available for closure",
            snapshot.capacity)
    }

    /// Number of configured UUIDs, including any not yet stored. This is an
    /// admission setting, never a claim about browser residency or availability.
    pub fn configured_count(&self) -> Option<usize> {
        match &self.config.read().expect("pool config poisoned").eligible_accounts {
            EligibleAccounts::All(_) => None,
            EligibleAccounts::Only(ids) => Some(ids.len()),
        }
    }

    pub(super) fn require(&self, id: &AccountId) -> Result<(), ApiError> {
        if self.allows(id) {
            Ok(())
        } else {
            Err(ApiError::ServingCapacity(
                "account is disabled by the pool configuration; its saved session is retained".into(),
            ))
        }
    }

    fn no_eligible_account(&self, pool: &[ManagedAccount]) -> ApiError {
        if !pool.is_empty() {
            let causes: std::collections::BTreeSet<_> = pool.iter()
                .filter_map(ManagedAccount::eligibility_problem).collect();
            ApiError::ServingCapacity(causes.into_iter().collect::<Vec<_>>().join("; "))
        } else if self.configured_count().is_some() {
            ApiError::ServingCapacity("no saved accounts match the configured account allowlist".into())
        } else {
            ApiError::Internal("no saved accounts are available".into())
        }
    }

    /// Keep explicit selection semantics: never substitute another identity
    /// when a caller asks for an unknown or parked account.
    pub(super) fn select(
        &self,
        pool: Vec<ManagedAccount>,
        requested: Option<&str>,
    ) -> Result<ManagedAccount, ApiError> {
        self.select_at(pool, requested, framer_core::rotation::now_ms())
    }

    fn select_at(
        &self,
        mut pool: Vec<ManagedAccount>,
        requested: Option<&str>,
        now: u64,
    ) -> Result<ManagedAccount, ApiError> {
        if let Some(id) = requested {
            let account = pool.into_iter().find(|a| a.id.0.to_string() == id)
                .ok_or_else(|| ApiError::Internal("unknown account id".into()))?;
            self.require(&account.id)?;
            if let Some(until) = account.cooling_until_ms.filter(|until| *until > now) {
                return Err(Self::cooling(until, now));
            }
            let snapshot = self.residents.snapshot()
                .map_err(|error| ApiError::PoolBusy(error.to_string()))?;
            let protected = self.mutations.lock()
                .map_err(|_| ApiError::PoolBusy("account management is unavailable".into()))?;
            if !Self::residency_allows(&snapshot, &account.id, &protected) {
                return Err(ApiError::PoolBusy(Self::residency_failure(
                    &snapshot, &account.id, &protected)));
            }
            if let Some(cause) = account.eligibility_problem() {
                return Err(ApiError::ServingCapacity(cause));
            }
            return Ok(account);
        }
        // Retain preserves input order. The existing selector still owns all
        // health, cooldown, workspace saturation and round-robin decisions.
        pool.retain(|a| self.allows(&a.id));
        self.pick_ready(&pool, now)
    }

    pub(super) fn select_fresh(
        &self,
        pool: Vec<ManagedAccount>,
        excluded: &[String],
    ) -> Result<ManagedAccount, ApiError> {
        self.select_fresh_at(pool, excluded, framer_core::rotation::now_ms())
    }

    fn select_fresh_at(
        &self,
        mut pool: Vec<ManagedAccount>,
        excluded: &[String],
        now: u64,
    ) -> Result<ManagedAccount, ApiError> {
        let cooling_until = pool.iter()
            .filter(|a| self.allows(&a.id) && a.is_eligible())
            .filter_map(|a| a.cooling_until_ms.filter(|until| *until > now)).min();
        pool.retain(|a| self.allows(&a.id) && !excluded.contains(&a.id.0.to_string()));
        if pool.is_empty() {
            if let Some(until) = cooling_until {
                return Err(Self::cooling(until, now));
            }
            return Err(if self.configured_count().is_some() {
                ApiError::ServingCapacity(
                    "serving pool exhausted: no untried account in the pool configuration".into(),
                )
            } else {
                ApiError::Internal("pool exhausted: every account was refused for this request".into())
            });
        }
        self.pick_ready(&pool, now)
    }

    fn cooling(until: u64, now: u64) -> ApiError {
        ApiError::AccountCooling {
            retry_after_secs: until.saturating_sub(now).div_ceil(1000).max(1),
        }
    }

    fn pick_ready(&self, pool: &[ManagedAccount], now: u64) -> Result<ManagedAccount, ApiError> {
        let snapshot = self.residents.snapshot().map_err(|e| ApiError::PoolBusy(e.to_string()))?;
        let mutations = self.mutations.lock()
            .map_err(|_| ApiError::PoolBusy("account management is unavailable".into()))?.clone();
        let candidates: Vec<_> = pool.iter().filter(|account| !mutations.contains(&account.id)
            && Self::residency_allows(&snapshot, &account.id, &mutations))
            .cloned().collect();
        if candidates.is_empty() && !pool.is_empty() {
            let causes: std::collections::BTreeSet<_> = pool.iter().map(|account|
                Self::residency_failure(&snapshot, &account.id, &mutations)).collect();
            return Err(ApiError::PoolBusy(causes.into_iter().collect::<Vec<_>>().join("; ")));
        }
        let pool = candidates.as_slice();
        // The general selector can fall back to a cooling account. HTTP
        // admission must instead let the client back off before SSE starts.
        if !pool.iter().any(|a| a.is_available_at(now)) {
            if let Some(until) = pool.iter().filter(|a| a.is_eligible())
                .filter_map(|a| a.cooling_until_ms.filter(|until| *until > now)).min()
            {
                return Err(Self::cooling(until, now));
            }
        }
        self.rotation.pick_preferred_at(pool, now, |account| {
            // Reservations include preparation, queued work and cleanup.
            // Reuse a warm browser only when its load is otherwise equal.
            snapshot.residents.iter().find(|slot| slot.account == account.id)
                .map(|slot| (slot.users, false)).unwrap_or((0, true))
        }).cloned()
            .ok_or_else(|| self.no_eligible_account(pool))
    }
}

#[cfg(test)]
#[path = "serving_accounts_tests.rs"]
mod tests;
