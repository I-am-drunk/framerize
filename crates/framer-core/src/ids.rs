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

/// 9-char base62 id matching the real client's turnId/sessionId shape
/// (verified capture: "LHXgk7pUf"). Used for envelope ids.
pub fn mint_short_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static C: AtomicU64 = AtomicU64::new(0);
    let mut n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        ^ (std::process::id() as u128) << 64
        ^ (C.fetch_add(1, Ordering::Relaxed) as u128) << 96;
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut out = String::with_capacity(9);
    for _ in 0..9 {
        out.push(ALPHA[(n % 62) as usize] as char);
        n /= 62;
    }
    out
}
