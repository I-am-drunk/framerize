//! Environment registry: every FRAMERIZE_* knob in one place with typed
//! accessors, documented defaults, and spend-safety grouping.
//!
//! CONVENTIONS (2026-11-15 cleanup):
//! - `flag(NAME)` = opt-in, `== "1"` (default off).
//! - `kill_switch(NAME)` = opt-out, `!= "0"` (default on).
//! - `value(NAME)` = raw string for paths/URLs.
//! - Spend hierarchy: ALLOW_SPEND (master) > ALLOW_METERED / ALLOW_TOOL_LEGS
//!   (sub-keys). Tools mode is ONE knob with 3 states (see tools_mode()).
//! - Call sites should use these accessors, not std::env::var inline, so the
//!   full surface stays greppable here.

/// Opt-in flag, default off.
pub fn flag(name: &str) -> bool {
    std::env::var(name).map(|v| v == "1").unwrap_or(false)
}

/// Opt-out kill switch, default on.
pub fn kill_switch(name: &str) -> bool {
    std::env::var(name).map(|v| v != "0").unwrap_or(true)
}

/// Master spend opt-in.
pub fn allow_spend() -> bool {
    flag("FRAMERIZE_ALLOW_SPEND")
}

/// Metered direct-route opt-in (needs allow_spend too — two-key rule).
pub fn allow_metered() -> bool {
    flag("FRAMERIZE_ALLOW_METERED")
}

/// Deliberate goose route mount.
pub fn goose_route() -> bool {
    flag("FRAMERIZE_GOOSE_ROUTE")
}

/// Preserve caller provider case on cutoff legs (default: lowercase).
pub fn ssc_allow_caps() -> bool {
    flag("FRAMERIZE_SSC_ALLOW_CAPS")
}

/// Pump timing trace.
pub fn cut_trace() -> bool {
    flag("FRAMERIZE_CUT_TRACE")
}

/// Multi-tool early abort.
pub fn tool_cutoff() -> bool {
    flag("FRAMERIZE_TOOL_CUTOFF")
}

/// Learned model-gate BLOCKS (default: learn-only).
pub fn model_gate_blocks() -> bool {
    flag("FRAMERIZE_MODEL_GATE")
}

/// Thinking-cap opt-out (default: cap at 1024).
pub fn no_thinking_cap() -> bool {
    flag("FRAMERIZE_NO_THINKING_CAP")
}

/// Native tool-leg attempts (test accounts).
pub fn allow_tool_legs() -> bool {
    flag("FRAMERIZE_ALLOW_TOOL_LEGS")
}

/// Schema-as-prose tool emulation (experiments only).
pub fn emulate_tools() -> bool {
    flag("FRAMERIZE_EMULATE_TOOLS")
}

/// Tools mode: one knob, three states (Refuse default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolsMode {
    Refuse,
    Emulate,
    NativeAttempt,
}

/// Direct-nitro routing (default on; =0 restores cutoff-first).
pub fn direct_nitro() -> bool {
    kill_switch("FRAMERIZE_DIRECT_NITRO")
}

/// Per-dialect cutoff escape hatches (default: cutoff on).
pub fn engine_chat_cutoff() -> bool {
    kill_switch("FRAMERIZE_ENGINE_CHAT")
}

/// Per-dialect cutoff escape hatches (default: cutoff on).
pub fn engine_responses_cutoff() -> bool {
    kill_switch("FRAMERIZE_ENGINE_RESPONSES")
}

/// Per-dialect cutoff escape hatches (default: cutoff on).
pub fn engine_anthro_cutoff() -> bool {
    kill_switch("FRAMERIZE_ENGINE_ANTHRO")
}

/// Max measured spend per request before halt+withhold.
pub fn max_spend_per_request() -> f64 {
    std::env::var("FRAMERIZE_MAX_SPEND_PER_REQUEST")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.01)
}

/// Minimum workspace balance to serve.
pub fn bill_floor() -> f64 {
    std::env::var("FRAMERIZE_CREDIT_FLOOR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2.0)
}

/// Breaker latch file override (persistence across restarts).
pub fn breaker_file() -> Option<String> {
    std::env::var("FRAMERIZE_BREAKER_FILE").ok()
}

/// Refund sweeper journal path.
pub fn refund_journal() -> Option<String> {
    std::env::var("FRAMERIZE_REFUND_JOURNAL").ok()
}

/// Refund lab route enable.
pub fn refund_lab() -> bool {
    flag("FRAMERIZE_REFUND_LAB")
}

/// Daemon log directory.
pub fn log_dir() -> Option<String> {
    std::env::var("FRAMERIZE_LOG_DIR").ok()
}
