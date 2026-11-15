//! Error taxonomy + anthropic-route helpers: upstream refusal classes
//! (castle/plan-gate/guardrail), retry policy, admission error rendering,
//! and the responses→messages fallback. Pure move out of lib.rs
//! (no behavior change).

use super::{pool_control, AppState, ApiError};
use super::handlers::hdr;
use framer_core::error::AccountCooldown;
use framer_core::convert::{self, ClientDialect};
use serde_json::Value;

pub(super) fn is_castle_refusal(e: &str) -> bool {
    if e.starts_with("upstream 403") {
        return false;
    }
    e.ends_with("risk-refused")
}

pub(super) fn should_retry_account(error: &str) -> bool {
    // credits-exhausted (2026-11-09): the chain's scope died mid-request.
    // The account's OTHER workspaces (or a fresh account's) may be funded;
    // the ledger cache dead-marks the drained scope on its next read, so the
    // re-pick routes around it. Without this, one drained workspace failed
    // the whole request in front of six funded accounts.
    // SPEND-TERMINAL NEVER REPICKS (2026-11-11 -- the 493->3 bleed): ledger-moved /
    // quarantined / spend-denied / breaker-tripped / credits-exhausted mean THIS request
    // already billed somewhere. A fresh account re-sends the full input and bills AGAIN
    // (input-side settlement at dispatch). Fail loud on the current account instead.
    // (credits-exhausted moves here from the old arm: same reason -- the scope that died
    // billed before dying, and the 2026-11-09 multi-workspace case is now covered by the
    // funded-scope picker routing around dead scopes on the NEXT request, not this one.)
    if framer_core::convert::is_spend_terminal(error) {
        return false;
    }
    // PLAN-GATE / GUARDRAIL NEVER REPICKS (2026-11-11 -- FOXTROT1 billed -0.40): an
    // entitlement refusal is deterministic per model, not per account. A fresh account
    // re-sends the full input and bills ANOTHER dispatch settle for an identical 403.
    // (The model allowlist already refuses known-gated ids pre-dispatch; this covers
    // newly-gated ones the allowlist hasn't learned yet.)
    if is_plan_gated(error) || is_guardrail_blocked(error) {
        return false;
    }
    is_castle_refusal(error) || AccountCooldown::from_error(error).is_some()
        || pool_control::cleanup_unconfirmed(error)
}

pub(super) fn chain_admission_error(error: ApiError) -> String {
    match error {
        ApiError::AccountCooling { retry_after_secs } => format!("account-cooling:{retry_after_secs}"),
        ApiError::Cooling { retry_after_secs } => format!("server-cooling:{retry_after_secs}"),
        ApiError::InvalidRequest(message) | ApiError::PoolBusy(message)
        | ApiError::Internal(message) | ApiError::ServingCapacity(message) => message,
    }
}

pub(super) fn is_plan_gated(e: &str) -> bool {
    e.contains("Upgrade your workspace to use this model") || e.contains("paid_plan_only_model")
}

pub(super) fn is_guardrail_blocked(e: &str) -> bool {
    e.contains("Model blocked by guardrail") || e.contains("0 endpoints out of")
}

pub(super) fn anthro_as_responses(a: &framer_core::convert::AssembledResponse) -> framer_core::convert::AssembledResponses {
    let text: String = a
        .content
        .iter()
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("");
    framer_core::convert::AssembledResponses {
        id: a.id.clone(),
        created_at: 0,
        output: vec![serde_json::json!({
            "id": a.id,
            "type": "message",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": text, "annotations": []}],
        })],
        status: "completed".into(),
        incomplete_details: None,
        usage: a.usage.clone(),
        service_tier: serde_json::Value::Null,
        input_tokens: a.input_tokens,
        output_tokens: a.output_tokens,
        chunks: a.chunks,
        metered_chunks: a.metered_chunks,
    }
}

pub(super) async fn anthro_route_fallback(
    s: &AppState,
    dialect: ClientDialect,
    payload: &serde_json::Value,
    cfg: &framer_core::port::ChainConfig,
    account: &framer_core::account::ManagedAccount,
    session: &framer_core::port::Session,
    project: &framer_core::ids::ProjectId,
    chat_session: &str,
) -> Result<framer_core::convert::AssembledResponses, String> {
    let anthro = convert::to_anthropic_request(dialect, payload)?;
    let assembled = s
        .client
        .freechat_long(
            session,
            s.browser.as_ref(),
            &account.id,
            project,
            anthro,
            cfg,
            chat_session,
            None,
        )
        .await?;
    Ok(anthro_as_responses(&assembled))
}

pub(super) fn anthro_model_for_log(dialect: ClientDialect, payload: &Value) -> String {
    let _ = dialect;
    convert::to_openrouter_id(payload["model"].as_str().unwrap_or_default())
}
