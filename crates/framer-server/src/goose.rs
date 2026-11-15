//! Goose route: the F27 paid-flagship recipe as a deliberate env-gated route.
//!
//! POST /goose/v1/complete {"model","prompt","max_tokens?"}
//! Mounted ONLY when FRAMERIZE_GOOSE_ROUTE=1 (boot-time gate). Unset or any
//! other value => route absent => 404. Disable = unset + restart.
//!
//! Thin shell over the REAL cutoff executor (cutoff_run), not a parallel
//! billing path: spend lock, breaker, account failover, quarantine, and
//! health marking are the same code as /api/custom/complete. The route adds:
//!  1. base id ALWAYS in the logs (one goose-target line per attempt:
//!     base_model verbatim + resolved_model + outcome + live delta);
//!  2. provider lowercased BEFORE the executor sees the id (F26 caps fix);
//!  3. bleed witness: live total before+after as live_delta in response+log.

use super::{custom::cutoff_run, AppState};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const GOOSE_ENV: &str = "FRAMERIZE_GOOSE_ROUTE";

pub fn enabled() -> bool {
    matches!(std::env::var(GOOSE_ENV).as_deref(), Ok("1"))
}

pub fn goose_routes(state: AppState) -> Router {
    Router::new()
        .route("/complete", axum::routing::post(goose_complete))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct GooseBody {
    #[serde(default)]
    model: String,
    #[serde(default)]
    prompt: String,
    // NOTE: no max_tokens in v1 — the cutoff executor has no output-cap
    // parameter (max_legs=1 still LegCaps into full essays, measured live).
    // Accepting a cap we don't enforce would lie. Follow-up: executor-side
    // stop/length directive work, then re-add the field.
}

#[derive(Debug, Serialize)]
struct GooseResp {
    outcome: String,
    text: String,
    total_debit: f64,
    req_id: String,
    base_model: String,
    resolved_model: String,
    live_delta: f64,
}

/// Provider lowercased BEFORE any gate (the F26 caps fix, in-route).
/// Suffix (:floor/:nitro) and core case are preserved for the executor.
pub fn normalize_goose_model(raw: &str) -> String {
    let trimmed = raw.trim();
    match trimmed.split_once('/') {
        Some((p, rest)) => format!("{}/{}", p.to_ascii_lowercase(), rest),
        None => trimmed.to_string(),
    }
}

/// Live-total read for the bleed witness: (total, source). Source is
/// "live" (the 60s live-balance cache — same read the funded-scope picker
/// uses) or "snapshot" (stored account credits, incl. dead-account phantom).
/// A delta is ONLY meaningful when both reads share the source; the handler
/// emits NaN otherwise (mixed-source deltas lie — measured live: 8339.07
/// snapshot vs 87.75 partial-live = -8251 garbage).
async fn live_total_cached(s: &AppState) -> (f64, &'static str) {
    let pool = match s.state.list().await {
        Ok(p) => p,
        Err(_) => return (f64::NAN, "error"),
    };
    let mut teams = std::collections::HashSet::new();
    for a in &pool {
        for sc in a.all_scopes() {
            teams.insert(sc.team.0.clone());
        }
    }
    let mut total = 0.0;
    let mut any = false;
    for t in teams {
        if let Some(b) = framer_core::rotation::cached_live_balance(&t) {
            total += b;
            any = true;
        }
    }
    if any {
        return (total, "live");
    }
    // Cold cache: stored snapshot (stale but present beats NaN).
    let mut snap = 0.0;
    let mut any_snap = false;
    for a in &pool {
        if let Some(c) = a.credit.as_ref() {
            snap += c.available;
            any_snap = true;
        }
    }
    if any_snap { (snap, "snapshot") } else { (f64::NAN, "error") }
}

#[allow(clippy::too_many_arguments)]
fn goose_log(req_id: &str, base: &str, resolved: &str, outcome: &str,
    debit: f64, before: f64, after: f64, src_before: &str, src_after: &str) {
    // Base id ALWAYS in the logs: eprintln! lands in the service journal
    // (tracing has no subscriber in this daemon — tracing::warn! would go
    // nowhere). Grep: journalctl -u framerize.service | grep '\[goose\]'.
    // Delta is valid only when both reads share the source (else NaN).
    let delta = if src_before == src_after { after - before } else { f64::NAN };
    eprintln!(
        "[goose] req={} base_model={} resolved_model={} outcome={} total_debit={:.6} live_before={:.2}({}) live_after={:.2}({}) live_delta={:.4}",
        req_id, base, resolved, outcome, debit, before, src_before, after, src_after, delta);
    framer_core::observability::Event::new("goose.attempt")
        .request_id(req_id)
        .field("base_model", base.to_string())
        .field("resolved_model", resolved.to_string())
        .field("outcome", outcome.to_string())
        .field("total_debit", debit)
        .field("live_before", before)
        .field("live_after", after)
        .field("live_delta", delta)
        .outcome(if debit < -1e-4 { "billed" } else { "ok" })
        .emit();
}

async fn goose_complete(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<GooseBody>,
) -> Response {
    let base_model = body.model.clone();
    if body.prompt.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "prompt is required"})),
        )
            .into_response();
    }
    let resolved = normalize_goose_model(&body.model);
    let goose_req_id = format!("goose-{}-{}", framer_core::rotation::now_ms(),
        &Uuid::new_v4().to_string()[..8]);
    let (live_before, src_before) = live_total_cached(&s).await;
    // Delta is valid only when both reads share the source (mixed sources lie).
    let delta = |after: f64, src_after: &str| -> f64 {
        if src_before == src_after { after - live_before } else { f64::NAN }
    };
    // Single attempt (max_legs=1), no auto-retry through quarantine (F52:
    // quarantine is the ONLY branch that moves money; refusals are $0).
    let request = framer_custom::ChainRequest::Prompt(body.prompt.clone());
    match cutoff_run(s.clone(), &headers, request, &resolved, Some(1), None).await {
        Ok(out) => {
            let (live_after, src_after) = live_total_cached(&s).await;
            goose_log(&out.req_id, &base_model, &resolved,
                &format!("{:?}", out.assembled.outcome),
                out.assembled.total_debit, live_before, live_after, src_before, src_after);
            if matches!(out.assembled.outcome, framer_custom::ChainOutcome::Quarantined) {
                return (
                    StatusCode::PAYMENT_REQUIRED,
                    Json(serde_json::json!({
                        "error": "goose route: leg settled nonzero and was quarantined (no free path this attempt)",
                        "outcome": format!("{:?}", out.assembled.outcome),
                        "text": "",
                        "total_debit": out.assembled.total_debit,
                        "req_id": out.req_id,
                        "base_model": base_model,
                        "resolved_model": resolved,
                        "live_delta": delta(live_after, src_after),
                    })),
                )
                    .into_response();
            }
            Json(GooseResp {
                outcome: format!("{:?}", out.assembled.outcome),
                text: out.assembled.text,
                total_debit: out.assembled.total_debit,
                req_id: out.req_id,
                base_model,
                resolved_model: resolved,
                live_delta: delta(live_after, src_after),
            })
            .into_response()
        }
        Err(e) => {
            // Refusal path: no leg billed. Still one goose log line with the
            // base id.
            let (live_after, src_after) = live_total_cached(&s).await;
            goose_log(&goose_req_id, &base_model, &resolved, "refused", 0.0,
                live_before, live_after, src_before, src_after);
            e.into_response()
        }
    }
}

#[cfg(test)]
mod goose_tests {
    use super::*;

    #[test]
    fn provider_normalization_never_skips_the_gate_shape() {
        assert_eq!(normalize_goose_model("ANTHROPIC/claude-opus-5.5:nitro"), "anthropic/claude-opus-5.5:nitro");
        assert_eq!(normalize_goose_model("Anthropic/claude-opus-5.5:floor"), "anthropic/claude-opus-5.5:floor");
        assert_eq!(normalize_goose_model("anthropic/claude-opus-5.5:nitro"), "anthropic/claude-opus-5.5:nitro");
        assert_eq!(normalize_goose_model("gpt-4.1-mini"), "gpt-4.1-mini");
        assert_eq!(normalize_goose_model("  ANTHROPIC/x  "), "anthropic/x");
    }

    #[test]
    fn gate_is_off_unless_deliberately_enabled() {
        std::env::remove_var(GOOSE_ENV);
        assert!(!enabled());
        std::env::set_var(GOOSE_ENV, "0");
        assert!(!enabled());
        std::env::set_var(GOOSE_ENV, "yes");
        assert!(!enabled());
        std::env::set_var(GOOSE_ENV, "1");
        assert!(enabled());
        std::env::remove_var(GOOSE_ENV);
    }
}
