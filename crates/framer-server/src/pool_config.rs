use framer_core::ids::AccountId;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const POOL_SETTING_KEY: &str = "pool.v1";
pub const DEFAULT_ACTIVE_CAPACITY: usize = 6;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EligibleAccounts {
    All(AllAccounts),
    Only(Vec<AccountId>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AllAccounts { All }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolConfig {
    pub active_capacity: usize,
    pub eligible_accounts: EligibleAccounts,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self { active_capacity: DEFAULT_ACTIVE_CAPACITY,
            eligible_accounts: EligibleAccounts::All(AllAccounts::All) }
    }
}

impl PoolConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=64).contains(&self.active_capacity) {
            return Err("active_capacity must be between 1 and 64".into());
        }
        if let EligibleAccounts::Only(ids) = &self.eligible_accounts {
            if ids.iter().collect::<HashSet<_>>().len() != ids.len() {
                return Err("eligible_accounts contains duplicate UUIDs".into());
            }
        }
        Ok(())
    }

    pub fn allows(&self, id: &AccountId) -> bool {
        match &self.eligible_accounts {
            EligibleAccounts::All(_) => true,
            EligibleAccounts::Only(ids) => ids.contains(id),
        }
    }
}
