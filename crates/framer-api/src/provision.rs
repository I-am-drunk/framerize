//! Provisioning primitives (2026-10-05): when an account is added, claim its
//! FULL capacity before admission runs:
//!
//!   - GRANT CLAIM: Framer grants 500 free-intro credits to the first TWO
//!     workspaces on an account (verified live 2026-10-05: ws1=500, ws2=500,
//!     ws3=0 on a current signup). `claim_workspace_grant` creates the second
//!     workspace when the account has just one, so admission sees its grant.
//!   - PROJECT ENSURE: every AI call needs a projectId in the primary
//!     workspace. `ensure_project` adopts the workspace's newest project or
//!     creates one (GET framer.com/projects/new?team=<spaceId> → 307 Location
//!     carries the new bare id; the /account/projects POST 404s for workspace
//!     spaces — the page route is the real creation path).

use crate::credits;
use crate::transport::{Auth, Transport};
use framer_core::ids::{ProjectId, TeamId};
use framer_core::port::Session;

#[cfg(test)]
#[path = "project_creation_deadline_tests.rs"]
mod deadline_tests;

#[cfg(test)]
#[path = "project_adoption_scope_tests.rs"]
mod adoption_scope_tests;

fn project_client_builder(
    connect_timeout: std::time::Duration,
    request_timeout: std::time::Duration,
) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(connect_timeout)
        .timeout(request_timeout)
}

/// Create the account's second workspace if it has fewer than two, claiming
/// its 500-credit grant. Returns Ok(true) when a workspace was created.
/// Best-effort: failures are logged, never fatal — admission decides whether
/// the account is usable.
pub async fn claim_workspace_grant(t: &Transport, session: &Session) -> Result<bool, String> {
    let me = credits::me(t, session).await?;
    let orgs: Vec<&serde_json::Value> = me["spaces"]
        .as_array()
        .map(|a| a.iter().filter(|sp| sp["scope"].as_str() == Some("organization")).collect())
        .unwrap_or_default();
    if orgs.len() >= 2 {
        return Ok(false);
    }
    let name = format!("Workspace {}", orgs.len() + 1);
    match t
        .post(
            "/account/teams",
            Auth::Cookie(session),
            serde_json::json!({ "teamName": name }),
        )
        .await
    {
        Ok(_) => {
            eprintln!("[provision] claimed workspace grant: created '{name}'");
            Ok(true)
        }
        Err(e) => {
            eprintln!("[provision] workspace create failed (non-fatal): {e}");
            Ok(false)
        }
    }
}

/// Adopt the workspace's newest project, or create one when it has none.
pub async fn ensure_project(
    t: &Transport,
    session: &Session,
    team: &TeamId,
    space_id: &str,
) -> Result<ProjectId, String> {
    let v = t
        .get(
            &format!(
                "/web/v2/dashboard/teams/{}?collectionId=recent&orderBy=lastOpenedAt&limit=5",
                team.0
            ),
            Auth::Cookie(session),
        )
        .await?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(project) = project_to_adopt(&v, team)? {
        return Ok(project);
    }
    create_project(session, space_id).await
}

fn project_to_adopt(
    response: &serde_json::Value,
    team: &TeamId,
) -> Result<Option<ProjectId>, String> {
    let Some(project) = response["projects"].as_array().and_then(|ps| ps.first()) else {
        return Ok(None);
    };
    let Some(id) = project["id"].as_str() else {
        return Ok(None);
    };
    for metadata in [response, project] {
        if let Some(value) = metadata.get("teamId") {
            if value.as_str() != Some(team.0.as_str()) {
                return Err("project adoption response has incompatible teamId".into());
            }
        }
    }
    Ok(Some(ProjectId(id.to_string())))
}

/// GET framer.com/projects/new?team=<spaceId> WITHOUT following redirects;
/// the 307 Location header carries the new bare project id.
async fn create_project(session: &Session, space_id: &str) -> Result<ProjectId, String> {
    let client = project_client_builder(
        std::time::Duration::from_secs(10),
        std::time::Duration::from_secs(30),
    )
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(format!("https://framer.com/projects/new?team={space_id}"))
        .header("cookie", &session.cookie_header)
        .header(
            "user-agent",
            "Mozilla/5.0 (X11; Linux x86_64; rv:152.0) Gecko/20100101 Firefox/152.0",
        )
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let loc = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .ok_or("project create returned no location")?;
    let last = loc.rsplit('/').next().unwrap_or(loc);
    let id = if last.contains("--") {
        last.split("--").nth(1).and_then(|s| s.split('-').next()).unwrap_or(last)
    } else {
        last
    };
    let id = id.trim();
    if id.len() < 15 {
        return Err(format!("project create returned odd location: {loc}"));
    }
    Ok(ProjectId(id.to_string()))
}
