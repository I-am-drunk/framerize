//! Workspace management. READS are mapped; CREATION is not.
//!
//! Mapped:
//!   - workspace list comes from GET /web/users/me → spaces[] (scope "user" =
//!     personal, scope "organization" = workspace). The credit-bearing team is
//!     `spaces[].ownerId` — NOT the space id. Verified live 2026-10-01: the
//!     ai-credits endpoint 403s for space ids and for the personal space's owner
//!     (the user id), 200 for the workspace's owner team.
//!
//! Not mapped:
//!   - the create-team endpoint. TeamCreationModal receives `api.createTeam` from
//!     its host bundle; capture it live (dashboard → workspace menu → Add
//!     workspace). Until then this returns an error and rotation is limited to
//!     pre-existing teams.

use crate::transport::Transport;
use framer_core::ids::TeamId;
use framer_core::port::{Session, WorkspaceSpec};

pub async fn create(_t: &Transport, _session: &Session, _spec: &WorkspaceSpec) -> Result<TeamId, String> {
    Err("create-team endpoint not yet captured — add a workspace in the Framer UI, or capture the call".to_string())
}
