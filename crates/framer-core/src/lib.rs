//! Stable domain + orchestration. No HTTP, no SQL, no Framer quirks — those live
//! behind the ports in `port.rs` and are implemented in `framer-api` / `framer-store`.

pub mod observability;
pub mod account;
pub mod account_lease;
pub mod browser_lifecycle;
pub mod resident_pool;
pub mod operation_gate;
#[cfg(feature = "native-dispatch")]
pub mod native_dispatch;
pub mod convert;
pub mod credits;
pub mod error;
pub mod gate;
pub mod ids;
pub mod pipeline;
pub mod port;
pub mod native_device;
pub mod rotation;
pub mod wave;
pub mod workspace;

pub use account::{AccountState, ManagedAccount, SessionHealth};
pub use error::CoreError;
pub use ids::{AccountId, FramerUserId, ProjectId, TeamId};
pub use pipeline::{InferenceOutcome, InferenceRequest, InferenceResult, Usage};
pub use port::{BrowserBridge, CastleToken, FramerClient, Session, SessionStore, StateStore, WorkspaceSpec};
pub use workspace::{CreditSnapshot, Workspace};
