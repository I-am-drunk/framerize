//! Custom-route admin surface: status dashboard, refund lab, sweep/claim probes,
//! mock transport, refund sweeper journal. Pure move out of custom.rs (no
//! behavior change). The serving path
//! (custom_complete_handler -> custom_real -> cutoff_run -> custom_real_once)
//! stays in custom.rs untouched.

use super::{custom::{CompleteBody, chain_step, custom_complete_handler}, ApiError, AppState};
use super::pool_control::{lease_account, RequestResidency};
use crate::{retain_owner, AbortOnDrop};
use async_trait::async_trait;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use framer_core::ids::{AccountId, ProjectId};
use framer_core::port::{BrowserBridge, Session};
use serde::Deserialize;
use std::sync::Arc;

pub fn custom_routes(state: AppState) -> Router {
    Router::new()
        .route("/complete", post(custom_complete_handler))
        .route(
            "/complete/",
            post(custom_complete_handler).get(hint_handler).put(hint_handler),
        )
        .route("/complete/status", axum::routing::get(status_handler).post(status_reset_handler))
        .route("/refund_lab", axum::routing::post(refund_lab_handler))
        .route("/sweep_probe", axum::routing::post(sweep_probe_handler))
        .route("/claim_grant", axum::routing::post(claim_grant_handler))
        .with_state(state)
}

async fn sweep_probe_handler(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let acct = headers.get("x-framerize-account").and_then(|v| v.to_str().ok()).map(|x| x.to_string());
    let sel = crate::select_funded_account(&s, acct).await;
    let (account, _res, session, scope) = match sel {
        Ok(t) => t,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": format!("no funded: {e:?}")}))).into_response(),
    };
    let bearer = match s.client.custom_bearer(&session, &account.id).await {
        Ok(b) => b,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": format!("bearer: {e}")}))).into_response(),
    };
    if s.browser.seed_session(&account.id, &session.cookie_header).await.is_err() {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": "seed fail"}))).into_response()
    };
    // Fully-flexible billing probe: control the upstream `path` and the raw native
    // `payload` (or build from model/prompt/max_tokens). `stream_mode` controls how the
    // stream key is sent: "false" (explicit), "true", "omit" (key absent), "null", "string".
    // Measures the real ledger delta across settle_secs. Optional refund (do_refund).
    let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("/ai/v3/chat/").to_string();
    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("anthropic/claude-fable-5.2").to_string();
    let stream_mode = body.get("stream_mode").and_then(|v| v.as_str()).unwrap_or("false").to_string();
    let max_tokens = body.get("max_tokens").and_then(|v| v.as_u64()).unwrap_or(16) as u32;
    let do_refund = body.get("do_refund").and_then(|v| v.as_bool()).unwrap_or(false);
    let settle_secs = body.get("settle_secs").and_then(|v| v.as_u64()).unwrap_or(75);
    let prompt = body.get("prompt").and_then(|v| v.as_str()).unwrap_or("Say ok").to_string();
    let n = body.get("n").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let custom_payload = body.get("payload").cloned();
    let feature_category = body.get("feature_category").and_then(|v| v.as_str()).unwrap_or("translation").to_string();
    let feature_name = body.get("feature_name").and_then(|v| v.as_str()).unwrap_or("translation").to_string();
    let step = body.get("step").and_then(|v| v.as_str()).unwrap_or("translate").to_string();
    let led0 = s.client.usage(&session, &scope.team).await.map(|x| x.available).unwrap_or(f64::NAN);
    let mut results: Vec<serde_json::Value> = Vec::new();
    for i in 0..n {
        let mut payload = custom_payload.clone().unwrap_or_else(|| serde_json::json!({"model": model, "messages": [{"role": "user", "content": prompt}], "max_tokens": max_tokens}));
        match stream_mode.as_str() {
            "true" => { payload["stream"] = serde_json::json!(true); }
            "omit" => { payload.as_object_mut().map(|o| o.remove("stream")); }
            "null" => { payload["stream"] = serde_json::Value::Null; }
            "string" => { payload["stream"] = serde_json::json!("false"); }
            _ => { payload["stream"] = serde_json::json!(false); }
        }
        let sess = format!("probe-{}-{}", i, &uuid::Uuid::new_v4().simple().to_string()[..6]);
        let envelope = if let Some(tid) = body.get("turn_id").and_then(|v| v.as_str()) {
            framer_api::proxy::envelope_raw4(&scope.project, &feature_category, &feature_name, &step, &model, &payload, &sess, 0, Some(tid))
        } else {
            framer_api::proxy::envelope_raw3(&scope.project, &feature_category, &feature_name, &step, &model, &payload, &sess, 0)
        };
        let turn = envelope["turnId"].as_str().unwrap_or("").to_string();
        let serve = s.browser.ai_post(&account.id, &path, &bearer, &envelope).await;
        let (st, uh, txt) = match serve {
            Ok(t) => t,
            Err(e) => { results.push(serde_json::json!({"i": i, "serve_ok": false, "err": e})); continue; }
        };
        let snippet: String = txt.chars().take(4000).collect();
        let mut row = serde_json::json!({"i": i, "serve_status": st, "usage_hdr": uh, "turn": &turn[..8.min(turn.len())], "turn_full": turn, "body": snippet});
        if do_refund {
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
            let rbody = serde_json::json!({"projectId": scope.project.0.as_str(), "featureCategory": feature_category.as_str(), "featureName": feature_name.as_str(), "step": step.as_str(), "sessionId": sess.as_str(), "stepIndex": 0, "turnId": turn.as_str(), "userPrompt": "", "castleToken": "garbage"});
            match s.browser.ai_post(&account.id, "/ai/v3/refund/", &bearer, &rbody).await {
                Ok((rst, _, rtxt)) => {
                    let granted = rtxt.contains("granted") && rtxt.contains("true");
                    row["granted"] = serde_json::json!(granted);
                    row["refund_status"] = serde_json::json!(rst);
                }
                Err(e) => { row["refund_err"] = serde_json::json!(e); }
            }
        }
        results.push(row);
    }
    tokio::time::sleep(std::time::Duration::from_secs(settle_secs)).await;
    let led1 = s.client.usage(&session, &scope.team).await.map(|x| x.available).unwrap_or(f64::NAN);
    Json(serde_json::json!({
        "account": &account.id.0.to_string()[..8], "team": &scope.team.0[..8], "model": model, "path": path, "stream_mode": stream_mode, "n": n,
        "results": results, "ledger_before": led0, "ledger_after": led1, "final_delta": led1 - led0,
    })).into_response()
}

async fn claim_grant_handler(
    State(s): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let acct = body.get("account_id").and_then(|v| v.as_str()).map(|x| x.to_string());
    let sel = crate::select_funded_account(&s, acct).await;
    let (account, _res, session, scope) = match sel {
        Ok(t) => t,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": format!("no funded: {e:?}")}))).into_response(),
    };
    let total_credits = |s: AppState, session: framer_core::port::Session| async move {
        let me = match s.client.me(&session).await { Ok(m) => m, Err(_) => return f64::NAN };
        let orgs: Vec<&serde_json::Value> = me["spaces"].as_array()
            .map(|a| a.iter().filter(|sp| sp["scope"].as_str() == Some("organization")).collect())
            .unwrap_or_default();
        let mut total = 0.0;
        for sp in orgs {
            if let Some(tid) = sp["ownerId"].as_str() {
                let team = framer_core::ids::TeamId(tid.to_string());
                if let Ok(snap) = s.client.usage(&session, &team).await { total += snap.available; }
            }
        }
        total
    };
    let before = total_credits(s.clone(), session.clone()).await;
    let claimed = s.client.claim_workspace_grant(&session).await;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let after = total_credits(s.clone(), session.clone()).await;
    Json(serde_json::json!({
        "account": &account.id.0.to_string()[..8],
        "team": &scope.team.0[..8.min(scope.team.0.len())],
        "claimed": format!("{:?}", claimed),
        "credits_before": before,
        "credits_after": after,
        "delta": after - before,
    })).into_response()
}

async fn hint_handler() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(serde_json::json!({
            "error": "use POST /api/custom/complete with JSON {prompt, model, max_legs}",
        })),
    )
        .into_response()
}

async fn status_handler() -> Response {
    Json(serde_json::json!({
        "custom_route": "ok",
        "spend_lock": {
            "allowlist": ["gpt-4.1-mini", "claude-haiku-4.5", "gpt-6-astra"],
            "breaker_tripped": framer_custom::ledger::spend_breaker_tripped(),
        },
    }))
    .into_response()
}

async fn refund_lab_handler(
    State(s): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if std::env::var("FRAMERIZE_REFUND_LAB").map(|v| v == "1").unwrap_or(false) == false {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "refund lab disabled (FRAMERIZE_REFUND_LAB=1 to enable)"})),
        )
            .into_response();
    }
    let turn_id = body.get("turn_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if turn_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "turn_id required"})),
        )
            .into_response();
    }
    // Reuse the funded-account path (same account the turn settled on, by operator choice).
    let account_hdr = body.get("account").and_then(|v| v.as_str()).map(|x| x.to_string());
    let sel = crate::select_funded_account(&s, account_hdr).await;
    let (account, _residency, session, scope) = match sel {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": format!("no funded account: {e:?}")})),
            )
                .into_response();
        }
    };
    let fresh = body.get("fresh").and_then(|v| v.as_bool()).unwrap_or(false);
    let bearer_res = if fresh {
        s.client.custom_bearer_fresh(&session, &account.id).await
    } else {
        s.client.custom_bearer(&session, &account.id).await
    };
    let bearer = match bearer_res {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": format!("bearer mint failed: {e}")})),
            )
                .into_response();
        }
    };
    if s.browser.seed_session(&account.id, &session.cookie_header).await.is_err() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "session seed failed"})),
        )
            .into_response();
    }
    let project = scope.project.clone();
    let category = body.get("category").and_then(|v| v.as_str()).unwrap_or("translation").to_string();
    let feature = body.get("feature").and_then(|v| v.as_str()).unwrap_or("translation").to_string();
    let fname = body.get("feature_name").and_then(|v| v.as_str()).unwrap_or(&feature).to_string();
    let step = body.get("step").and_then(|v| v.as_str()).map(|x| x.to_string()).unwrap_or_else(|| chain_step(&category).to_string());
    let session_id = body.get("session_id").and_then(|v| v.as_str()).unwrap_or("lab").to_string();
    let step_index = body.get("step_index").and_then(|v| v.as_u64()).unwrap_or(0);
    let rbody = serde_json::json!({
        "projectId": project.0.as_str(),
        "featureCategory": category.as_str(),
        "featureName": fname.as_str(),
        "step": step.as_str(),
        "sessionId": session_id.as_str(),
        "stepIndex": step_index,
        "turnId": turn_id.as_str(),
        "userPrompt": "",
        "castleToken": "garbage",
    });
    match s.browser.ai_post(&account.id, "/ai/v3/refund/", &bearer, &rbody).await {
        Ok((status, _, text)) => {
            let granted = text.contains("\"granted\":true") || text.contains("\"granted\": true");
            eprintln!("[refund-lab] turn {} session={} step={} feature={} -> HTTP {} granted={} body={}",
                &turn_id[..8.min(turn_id.len())], session_id, step, fname, status, granted,
                &text[..text.len().min(200)]);
            Json(serde_json::json!({
                "turn": &turn_id[..8.min(turn_id.len())],
                "status": status,
                "granted": granted,
                "session_id": session_id,
                "step": step,
                "body": &text[..text.len().min(500)],
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": format!("transport: {e}")})),
        )
            .into_response(),
    }
}

async fn status_reset_handler(Json(body): Json<serde_json::Value>) -> Response {
    if body.get("reset_spend_breaker").and_then(|v| v.as_bool()) == Some(true) {
        framer_custom::ledger::reset_spend_breaker();
        return Json(serde_json::json!({"spend_breaker": "reset"})).into_response();
    }
    Json(serde_json::json!({
        "custom_route": "ok",
        "spend_lock": {
            "allowlist": ["gpt-4.1-mini", "claude-haiku-4.5", "gpt-6-astra"],
            "breaker_tripped": framer_custom::ledger::spend_breaker_tripped(),
        },
    }))
    .into_response()
}






