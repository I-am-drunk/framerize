//! Dedupe previews keep full identities and never perform external I/O.
use super::*;

async fn preview(rows: Vec<ManagedAccount>) -> Value {
    let (state, ports) = fixture(false);
    let before = serde_json::to_value(&rows).unwrap();
    *ports.records.lock().unwrap() = rows;
    *ports.native_session.lock().unwrap() = Some(None);
    let config = state.serving_accounts.config();
    let outcome = crate::dedupe_accounts_observed(
        State(state.clone()),
        Json(crate::DedupeBody { apply: false }),
    )
    .await;
    assert!(outcome.cleanup_confirmed);
    let report = outcome.result.unwrap().0;
    assert_eq!(report["applied"], false);
    assert_eq!(
        serde_json::to_value(&*ports.records.lock().unwrap()).unwrap(),
        before
    );
    assert_eq!(state.serving_accounts.config(), config);
    assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.client_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.state_writes.load(Ordering::SeqCst), 0);
    assert_eq!(ports.state_reads.load(Ordering::SeqCst), 1);
    assert!(ports.saved.lock().unwrap().is_empty());
    assert!(ports.registered_proxies.lock().unwrap().is_empty());
    assert!(state.serving_accounts.mutations.lock().unwrap().is_empty());
    report
}

#[tokio::test(flavor = "current_thread")]
async fn short_device_ids_preserve_records_in_preview() {
    for device in ["", "x", "short", "1234567"] {
        let mut row = account(1);
        row.castle_device_id = device.into();
        let report = preview(vec![row]).await;
        assert_eq!(
            report["keeping"],
            json!([{
                "name": "fixture-1@example.invalid", "team": "team-1", "device": device
            }])
        );
        assert_eq!(report["removing"], json!([]));
        assert_eq!(report["needs_project"], json!([]));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn device_preview_stops_at_a_utf8_boundary() {
    for (device, expected) in [
        ("1234567é-tail", "1234567"),
        ("abc🚀é-tail", "abc🚀"),
        ("界界界", "界界"),
        ("ééééé", "éééé"),
        ("🚀🚀🚀", "🚀🚀"),
    ] {
        let mut row = account(1);
        row.castle_device_id = device.into();
        let report = preview(vec![row]).await;
        assert_eq!(report["keeping"][0]["device"], expected);
        assert_eq!(report["removing"], json!([]));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn team_preview_stops_at_a_utf8_boundary() {
    for (team, expected) in [
        ("1234567é-tail", "1234567"),
        ("abc🚀é-tail", "abc🚀"),
        ("界界界", "界界"),
        ("ééééé", "éééé"),
        ("é", "é"),
    ] {
        let mut row = account(1);
        row.current_team = Some(TeamId(team.into()));
        let report = preview(vec![row]).await;
        assert_eq!(report["keeping"][0]["team"], expected);
        assert_eq!(report["keeping"][0]["device"], "device-1");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn ascii_and_absent_team_previews_stay_compatible() {
    for (team, expected) in [
        (Some("team-abcdef"), Some("team-abc")),
        (Some("tiny"), Some("tiny")),
        (Some(""), Some("")),
        (None, None),
    ] {
        let mut row = account(1);
        row.castle_device_id = "device-abcdefgh".into();
        row.current_team = team.map(|id| TeamId(id.into()));
        let report = preview(vec![row]).await;
        assert_eq!(
            report["keeping"],
            json!([{
                "name": "fixture-1@example.invalid", "team": expected, "device": "device-a"
            }])
        );
        assert_eq!(report["needs_project"], json!([]));
        assert_eq!(report["removing"], json!([]));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn full_device_identity_drives_clone_and_dead_classification() {
    let mut first = account(1);
    first.castle_device_id = "abcdefgh-original".into();
    let mut clone = account(2);
    clone.castle_device_id = first.castle_device_id.clone();
    let mut sibling = account(3);
    sibling.castle_device_id = "abcdefgh-distinct".into();
    let mut unprovisioned = account(4);
    unprovisioned.current_project = None;
    unprovisioned.credit = Some(CreditSnapshot {
        available: 100.0,
        funds: 1,
        grants: 1,
        allocated: 100.0,
    });
    let mut dead = account(5);
    dead.current_project = None;
    let report = preview(vec![first, clone, sibling, unprovisioned, dead]).await;
    let mut keeping = report["keeping"].as_array().unwrap().clone();
    keeping.sort_by_key(|row| row["name"].as_str().unwrap().to_string());
    assert_eq!(
        keeping,
        vec![
            json!({"name": "fixture-1@example.invalid", "team": "team-1", "device": "abcdefgh"}),
            json!({"name": "fixture-3@example.invalid", "team": "team-3", "device": "abcdefgh"}),
            json!({"name": "fixture-4@example.invalid", "team": "team-4", "device": "device-4"}),
        ]
    );
    assert_eq!(report["needs_project"].as_array().unwrap().len(), 1);
    assert_eq!(
        report["needs_project"][0]["name"],
        "fixture-4@example.invalid"
    );
    assert_eq!(report["needs_project"][0]["credits"], 100.0);
    assert_eq!(report["needs_project"][0]["has_session"], false);
    assert_eq!(
        report["removing"],
        json!([
            {
                "name": "fixture-2@example.invalid",
                "reason": "shares a Castle device id with another record"
            },
            {
                "name": "fixture-5@example.invalid",
                "reason": "no project, no credits, no session — nothing to recover"
            }
        ])
    );
}
