//! Custom-route engine: chained streaming-abort legs with a ledger oracle.
//!
//! Proven mechanism: every leg streams against a generous stop and aborts
//! mid-stream, so the usage-bearing terminal never arrives and the ledger
//! settles nothing. Races (terminal before stop) retry halved; debits
//! quarantine loudly. See each module's docs for its contract.

#![allow(dead_code)]
#![warn(missing_docs)]

pub mod stop;
pub mod merge;
pub mod gate;
pub mod transport;
pub mod rung;
pub mod ledger;
pub mod chain;
pub mod format;
pub mod live;
pub mod mock_util;

pub use gate::{validate_public_url, GateError, MAX_URL_LEN};
pub use ledger::{after_leg, is_settled, signed_delta, Policy, LEDGER_EPS};
pub use merge::{assemble, is_near_zero, trim_overlap, ASSEMBLE_WINDOW, MAX_SCAN, MIN_OVERLAP};
pub use stop::{clamped_stop, continue_suffix, halt_after_race, length_guarantee_suffix, DEFAULT_STOP, MAX_STOP, MIN_STOP};
pub use transport::{CustomTransport, Event};
pub use rung::{run_rung, RungResult, LEG_TIMEOUT, MAX_CAPTURE, MAX_SILENT_POLLS};
pub use chain::{complete, last_anchor_chars, Assembled, ChainOutcome, ChainSettings, Leg};
pub use live::{BridgeStreamTransport, PageStream, StreamBatch, DRAIN_POLL_INTERVAL, DRAIN_TIMEOUT, MAX_EMPTY_DRAINS};
pub use mock_util::ScriptedTransport;
pub use format::{dialect_from_path, format_tail, Dialect, FormatTail};

/// Owner tag for logs and route registration.
pub fn handler_name() -> &'static str {
    "framer-custom"
}
