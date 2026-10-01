use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Our own stable id for an account. Independent of Framer's user id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub Uuid);

impl AccountId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AccountId {
    fn default() -> Self {
        Self::new()
    }
}

/// Framer's own id for a signed-in user (uuid; from /web/users/me or the JWT sub).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FramerUserId(pub String);

/// A Framer workspace's credit ledger key. NOTE: credits attach to the TEAM that
/// owns the workspace space (`spaces[].ownerId` in /web/users/me), not the space id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TeamId(pub String);

/// A Framer project id (20-char base62, from /projects/<name>--<id>). Every agent
/// inference call is scoped to a project.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(pub String);
