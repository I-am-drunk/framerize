//! Account management surface: provisioning (extra scopes, claimed workspaces),
//! account admin (remove/dedupe/list), live balances, and the pool shape view.
//! Pure move out of lib.rs (no behavior change).

use super::{pool_control, AppState, ApiError};
use super::handlers::hdr;
use super::serving_accounts::ServingAccountPolicy;
use crate::{DedupeBody, RemoveBody};
use framer_core::account::ManagedAccount;
use framer_core::ids::{AccountId, TeamId};
use framer_core::pipeline::Deps;
use framer_core::port::Session;
use axum::extract::State;
use axum::Json;

pub(super) async fn populate_extra_scopes(
    s: &AppState,
    session: &framer_core::port::Session,
    account: &mut ManagedAccount,
) {
    let deps = Deps {
        client: s.client.as_ref(),
        browser: s.browser.as_ref(),
        sessions: s.sessions.as_ref(),
        state: s.state.as_ref(),
    };
    let credited = match framer_core::credits::scan_workspaces(&deps, session).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[provision] extra-scope scan failed (non-fatal): {e}");
            return;
        }
    };
    let primary = account.current_team.as_ref().map(|t| t.0.clone());
    let mut scopes = Vec::new();
    for w in credited {
        if Some(w.team.0.clone()) == primary {
            continue; // the primary scope lives in current_team/current_project
        }
        if w.snapshot.available <= 0.0 {
            continue; // 0-credit workspaces 402 on every route — not a scope
        }
        let mut project = s
            .client
            .team_projects(session, &w.team)
            .await
            .ok()
            .and_then(|ps| ps.into_iter().next());
        if project.is_none() {
            match s.client.create_project(session, &w.team).await {
                Ok(p) => {
                    eprintln!("[provision] created project {} in extra workspace {}", p.0, &w.team.0[..8]);
                    project = Some(p);
                }
                Err(e) => {
                    eprintln!("[provision] project creation in extra workspace failed: {e}");
                    continue;
                }
            }
        }
        if let Some(project) = project {
            scopes.push(framer_core::account::WorkspaceScope { team: w.team.clone(), project });
        }
    }
    if !scopes.is_empty() {
        eprintln!("[provision] account now spans {} workspace scopes (multi-workspace concurrency)", scopes.len() + 1);
    }
    account.extra_workspaces = scopes;
}

pub(super) async fn claimed_workspaces(
    s: &AppState,
    exclude: AccountId,
) -> Result<std::collections::BTreeSet<String>, ApiError> {
    let pool = s.state.list().await.map_err(ApiError::Internal)?;
    Ok(pool
        .iter()
        .filter(|a| a.id != exclude)
        .filter_map(|a| a.current_team.as_ref().map(|t| t.0.clone()))
        .collect())
}

pub(super) async fn remove_account(
    State(s): State<AppState>,
    Json(body): Json<RemoveBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let _management = pool_control::management(&s, body.account_id, false).await?;
    let existing = s.state.get(body.account_id).await.map_err(ApiError::Internal)?;
    let Some(acc) = existing else {
        return Err(ApiError::Internal("account not found".into()));
    };
    s.serving_accounts.park_managed(s.browser.as_ref(), body.account_id).await?;
    if body.wipe_profile {
        // Best effort: a missing browser profile must not block the removal.
        let _ = s.browser.rotate_identity(&body.account_id).await;
    }
    s.state.remove(body.account_id).await.map_err(ApiError::Internal)?;
    Ok(Json(serde_json::json!({
        "status": "removed",
        "account": acc.email.unwrap_or(acc.alias),
    })))
}

pub(super) async fn dedupe_accounts(
    State(s): State<AppState>,
    Json(body): Json<DedupeBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let pool = s.state.list().await.map_err(ApiError::Internal)?;
    let now = framer_core::rotation::now_ms();

    // Device clones: keep the first record per device id, flag later ones.
    let mut seen_devices: std::collections::BTreeSet<String> = Default::default();
    let mut doomed: Vec<(ManagedAccount, &'static str)> = Vec::new();
    let mut keeping = Vec::new();
    let mut needs_project: Vec<serde_json::Value> = Vec::new();

    // Stable order so the same record survives across repeated runs: usable
    // first, then one with credits, then fewest failures.
    let mut ordered = pool;
    ordered.sort_by(|a, b| {
        b.is_available_at(now)
            .cmp(&a.is_available_at(now))
            .then(
                b.credit
                    .as_ref()
                    .map(|c| c.available)
                    .unwrap_or(0.0)
                    .total_cmp(&a.credit.as_ref().map(|c| c.available).unwrap_or(0.0)),
            )
            .then(a.consecutive_failures.cmp(&b.consecutive_failures))
    });

    for a in ordered {
        let name = a.email.clone().unwrap_or_else(|| a.alias.clone());
        // Device clones first: that judgement needs no external state.
        if !seen_devices.insert(a.castle_device_id.clone()) {
            doomed.push((a, "shares a Castle device id with another record"));
            continue;
        }
        // Dead = nothing of value to recover. Credits or a live session make an
        // account worth provisioning rather than deleting.
        let has_credits = a.credit.as_ref().is_some_and(|c| c.available > 0.0);
        let has_session = s
            .sessions
            .load(&a.framer_user_id)
            .await
            .map_err(ApiError::Internal)?
            .is_some();
        if a.current_project.is_none() && !has_credits && !has_session {
            doomed.push((a, "no project, no credits, no session — nothing to recover"));
            continue;
        }
        if a.current_project.is_none() {
            // Kept, but flagged: it cannot serve traffic until it has a project.
            needs_project.push(serde_json::json!({
                "name": name,
                "credits": a.credit.as_ref().map(|c| c.available),
                "has_session": has_session,
                "reason": "no project yet — provision it, do not delete it",
            }));
        }
        keeping.push(serde_json::json!({
            "name": name,
            "team": a.current_team.as_ref().map(|t| t.0[..8.min(t.0.len())].to_string()),
            "device": a.castle_device_id[..8].to_string(),
        }));
    }

    let mut removed = Vec::new();
    for (dup, why) in doomed {
        let name = dup.email.clone().unwrap_or_else(|| dup.alias.clone());
        if body.apply {
            let _management = pool_control::management(&s, dup.id, false).await?;
            let current = s.state.get(dup.id).await.map_err(ApiError::Internal)?;
            let current = serde_json::to_value(current).map_err(|e| ApiError::Internal(e.to_string()))?;
            let expected = serde_json::to_value(Some(&dup)).map_err(|e| ApiError::Internal(e.to_string()))?;
            if current != expected {
                return Err(ApiError::PoolBusy("account changed during deduplication; retry the preview".into()));
            }
            s.serving_accounts.park_managed(s.browser.as_ref(), dup.id).await?;
            s.state.remove(dup.id).await.map_err(ApiError::Internal)?;
        }
        removed.push(serde_json::json!({ "name": name, "reason": why }));
    }

    Ok(Json(serde_json::json!({
        "applied": body.apply,
        "keeping": keeping,
        "needs_project": needs_project,
        "removing": removed,
        "note": "Same-login siblings are KEPT: each has its own Castle device id,                  and spreading mints across devices is what stops 403 storms. Only                  dead records (no project) and device clones are redundant.",
    })))
}

pub(super) async fn accounts(State(s): State<AppState>) -> Result<Json<Vec<ManagedAccount>>, ApiError> {
    Ok(Json(s.state.list().await.map_err(ApiError::Internal)?))
}

pub(super) async fn accounts_live(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let pool = s.state.list().await.map_err(ApiError::Internal)?;
    let residents = s.serving_accounts.residents.snapshot().map_err(|e| ApiError::Internal(e.to_string()))?;
    let floor: f64 = framer_core::rotation::bill_floor();
    // Sessions for every account up front (one store read each), so the live
    // reads below run in PARALLEL per workspace instead of serial per account.
    let mut sessions_by_user: std::collections::HashMap<String, framer_core::port::Session> =
        std::collections::HashMap::new();
    for account in &pool {
        let user = account.framer_user_id.0.clone();
        if sessions_by_user.contains_key(&user) {
            continue;
        }
        if let Ok(Some(sess)) = s.sessions.load(&account.framer_user_id).await {
            sessions_by_user.insert(user, sess);
        }
    }
    // Fetch every stale team's live balance in PARALLEL (one GET per team per
    // 60s, all at once) — the serial per-account read made the first cold call
    // take ~20s under 24 workspaces, hanging every consumer of this endpoint.
    let mut stale: Vec<(String, String)> = Vec::new(); // (user, team)
    for account in &pool {
        if !sessions_by_user.contains_key(&account.framer_user_id.0) {
            continue;
        }
        for sc in &account.all_scopes() {
            let team = sc.team.0.clone();
            if framer_core::rotation::cached_live_balance(&team).is_none() {
                stale.push((account.framer_user_id.0.clone(), team));
            }
        }
    }
    {
        let fetches = stale.into_iter().map(|(user, team)| {
            let sess = sessions_by_user.get(&user).cloned();
            let team2 = team.clone();
            let client = s.client.clone();
            async move {
                if let Some(sess) = sess {
                    if let Ok(snap) = client.usage(&sess, &framer_core::ids::TeamId(team.clone())).await {
                        framer_core::rotation::note_live_balance(&team2, snap.available);
                        framer_core::rotation::note_live_renewal(&team2, snap.renews_at.clone());
                    }
                }
            }
        });
        futures::future::join_all(fetches).await;
    }
    let mut out = Vec::new();
    for account in &pool {
        let scopes = account.all_scopes();
        let mut workspaces = Vec::new();
        let mut funded = 0usize;
        let mut unfunded = 0usize;
        for sc in &scopes {
            let team = sc.team.0.clone();
            let (balance, source) = match framer_core::rotation::cached_live_balance(&team) {
                Some(b) => (Some(b), "live-cache"),
                None => (account.credit.as_ref().map(|c| c.available), "cached"),
            };
            let saturated = framer_core::rotation::is_saturated(&team);
            let dead = matches!(balance, Some(b) if b < floor);
            let mark = if saturated {
                "saturated"
            } else if dead {
                "dead"
            } else if balance.is_some() {
                "funded"
            } else {
                "unknown"
            };
            if mark == "funded" { funded += 1; } else { unfunded += 1; }
            workspaces.push(serde_json::json!({
                "team": team,
                "project": sc.project.0,
                "balance": balance,
                "source": source,
                "mark": mark,
                // A drained workspace refills on this date (free grants renew
                // monthly) -- visible so "when does capacity return" is a
                // lookup, not a guess.
                "renews_at": framer_core::rotation::cached_renewal(&team),
            }));
        }
        let now = framer_core::rotation::now_ms();
        let cooling = account.cooling_until_ms.filter(|u| *u > now);
        let resident = residents.residents.iter().find(|r| r.account == account.id);
        let verdict = if scopes.is_empty() {
            "unused"
        } else if funded > 0 && cooling.is_none() {
            "active"
        } else {
            "skipped"
        };
        out.push(serde_json::json!({
            "id": account.id.0,
            "alias": account.alias,
            "state": account.state,
            "state_detail": format!("{:?}", account.state),
            "consecutive_failures": account.consecutive_failures,
            "cooling_until_ms": cooling,
            "resident": resident.map(|r| format!("{:?}", r.status)),
            "workspaces": workspaces,
            "funded_workspaces": funded,
            "unfunded_workspaces": unfunded,
            "verdict": verdict,
        }));
    }
    // Pool-level capacity at a glance: the question "how much runway is
    // left?" should never require reading every row. Total across cached
    // workspace balances + the soonest renewal (when the pool starts
    // refilling on its own).
    let mut total_available = 0.0f64;
    let mut funded_ws = 0u32;
    let mut dead_ws = 0u32;
    let mut soonest_renewal: Option<String> = None;
    for a in &out {
        for w in a["workspaces"].as_array().cloned().unwrap_or_default() {
            if let Some(b) = w["balance"].as_f64() {
                total_available += b;
                if w["mark"].as_str() == Some("funded") { funded_ws += 1; }
                if w["mark"].as_str() == Some("dead") { dead_ws += 1; }
            }
            if let Some(r) = w["renews_at"].as_str() {
                if soonest_renewal.as_deref().map(|s| r < s).unwrap_or(true) {
                    soonest_renewal = Some(r.to_string());
                }
            }
        }
    }
    Ok(Json(serde_json::json!({
        "floor": floor,
        "accounts": out,
        "pool": {
            "total_available": (total_available * 100.0).round() / 100.0,
            "funded_workspaces": funded_ws,
            "dead_workspaces": dead_ws,
            "soonest_renewal": soonest_renewal,
        },
    })))
}

pub(super) async fn pool_shape(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let pool = s.state.list().await.map_err(ApiError::Internal)?;
    let now = framer_core::rotation::now_ms();
    let residency = s.serving_accounts.residents.snapshot()
        .map_err(|e| ApiError::PoolBusy(e.to_string()))?;
    let protected = s.serving_accounts.mutations.lock()
        .map_err(|_| ApiError::PoolBusy("account management is unavailable".into()))?.clone();

    let mut teams: std::collections::BTreeMap<String, Vec<&ManagedAccount>> = Default::default();
    for a in &pool {
        let key = a.current_team.as_ref().map(|t| t.0.clone()).unwrap_or_else(|| "none".into());
        teams.entry(key).or_default().push(a);
    }
    let users: std::collections::BTreeSet<&str> =
        pool.iter().map(|a| a.framer_user_id.0.as_str()).collect();
    let devices: std::collections::BTreeSet<&str> =
        pool.iter().map(|a| a.castle_device_id.as_str()).collect();

    let workspaces: Vec<serde_json::Value> = teams
        .iter()
        .map(|(team, accounts)| {
            serde_json::json!({
                "team": team,
                "renews_at": framer_core::rotation::cached_renewal(team),
                "accounts": accounts.iter().map(|a| {
                    serde_json::json!({
                        "account_id": a.id,
                        "name": a.email.clone().unwrap_or_else(|| a.alias.clone()),
                        "state": format!("{:?}", a.state),
                        "serving_allowed": s.serving_accounts.allows(&a.id),
                        "available": s.serving_accounts.allows(&a.id) && a.is_available_at(now)
                            && ServingAccountPolicy::residency_allows(&residency, &a.id, &protected),
                        "cooling_ms_left": a.cooling_until_ms
                            .map(|u| u.saturating_sub(now)).filter(|m| *m > 0),
                    })
                }).collect::<Vec<_>>(),
                "credits": accounts.iter().find_map(|a| a.credit.as_ref().map(|c| c.available)),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "accounts": pool.len(),
        "available_now": pool.iter().filter(|a| s.serving_accounts.allows(&a.id)
            && a.is_available_at(now)
            && ServingAccountPolicy::residency_allows(&residency, &a.id, &protected)).count(),
        "serving_policy": {
            "mode": if s.serving_accounts.configured_count().is_some() { "runtime_allowlist" } else { "all_accounts" },
            "configured_accounts": s.serving_accounts.configured_count(),
            "allowed_records": pool.iter().filter(|a| s.serving_accounts.allows(&a.id)).count(),
            "parked_records": pool.iter().filter(|a| !s.serving_accounts.allows(&a.id)).count(),
            "browser_residency": "tracked_slots",
            "changes_require_restart": false,
        },
        "config": s.serving_accounts.config(),
        "residency": residency,
        // These three are the levers, in descending order of effect.
        "distinct_workspaces": teams.len(),
        "distinct_logins": users.len(),
        "distinct_devices": devices.len(),
        "workspaces": workspaces,
        "note": "Credits and the 429 concurrency cap are per WORKSPACE; Castle risk is per DEVICE. Accounts sharing a team share credits and one concurrency budget.",
    })))
}
