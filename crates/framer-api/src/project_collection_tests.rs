//! Inert JSON coverage for the team-bound dashboard project collector.
use super::collect_project_ids;
use framer_core::ids::TeamId;
use serde_json::{json, Value};

const FIRST: &str = "0123456789ABCDEFGHIJ";
const SECOND: &str = "abcdefghij0123456789";
const THIRD: &str = "ZYXWVUTSRQ9876543210";

fn collect_for(team: &str, body: Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_project_ids(&body, &TeamId(team.into()), &mut out);
    out.into_iter().map(|id| id.0).collect()
}

#[test]
fn matching_team_metadata_uses_the_requested_team() {
    for team in ["team-B", "team-C"] {
        let body = json!({"teamId": team, "projects": [
            {"id": FIRST, "name": "inert", "teamId": team},
            {"id": SECOND, "lastOpenedAt": 0, "teamId": team}
        ]});
        assert_eq!(collect_for(team, body), vec![FIRST.to_owned(), SECOND.to_owned()]);
    }
}

#[test]
fn mismatching_project_team_is_skipped() {
    let body = json!({"projects": [{"id": FIRST, "name": "inert", "teamId": "team-C"}]});
    assert!(collect_for("team-B", body).is_empty());
}

#[test]
fn contradictory_wrapper_prunes_absent_and_matching_descendants() {
    let body = json!({"collection": {"teamId": "team-C", "items": [
        {"id": FIRST, "name": "no child metadata"},
        {"id": SECOND, "name": "matching child", "teamId": "team-B"},
        {"teamId": "team-B", "items": [[{"id": THIRD, "lastOpenedAt": 0}]]}
    ]}});
    assert!(collect_for("team-B", body).is_empty());
}

#[test]
fn malformed_team_metadata_prunes_projects_and_their_descendants() {
    for invalid in [json!(null), json!(false), json!(7), json!([]), json!({}),
        json!(""), json!(" team-B ")] {
        let body = json!({"projects": [{
            "id": FIRST, "name": "invalid team", "teamId": invalid,
            "items": [
                {"id": SECOND, "name": "no metadata"},
                {"id": THIRD, "name": "matching metadata", "teamId": "team-B"}
            ]
        }]});
        assert!(collect_for("team-B", body).is_empty());
    }
}

#[test]
fn absent_metadata_keeps_route_bound_collection_shapes() {
    let project = json!({"id": FIRST, "name": "route-bound"});
    for body in [json!({"projects": [project.clone()]}),
        json!({"items": [project.clone()]}),
        json!({"collection": {"items": [project]}})] {
        assert_eq!(collect_for("team-B", body), vec![FIRST.to_owned()]);
    }
}

#[test]
fn mixed_siblings_keep_compatible_projects_in_order() {
    let body = json!({"items": [
        {"id": FIRST, "name": "before rejected wrapper", "teamId": "team-B"},
        {"teamId": "team-C", "projects": [{"id": THIRD, "name": "hidden"}]},
        {"teamId": "team-B", "items": [{"id": SECOND, "name": "matching wrapper"}]},
        {"teamId": null, "items": [{"id": THIRD, "name": "malformed wrapper"}]},
        {"id": THIRD, "lastOpenedAt": 0}
    ]});
    assert_eq!(collect_for("team-B", body),
        vec![FIRST.to_owned(), SECOND.to_owned(), THIRD.to_owned()]);
}

#[test]
fn accepted_projects_deduplicate_in_first_accepted_order() {
    let body = json!([
        {"id": FIRST, "name": "rejected first", "teamId": "team-C"},
        {"id": SECOND, "name": "first accepted"},
        {"id": FIRST, "name": "second accepted", "teamId": "team-B"},
        {"id": SECOND, "lastOpenedAt": 0, "teamId": "team-B"},
        {"id": FIRST, "name": "duplicate"}
    ]);
    assert_eq!(collect_for("team-B", body), vec![SECOND.to_owned(), FIRST.to_owned()]);
}

#[test]
fn existing_project_shape_checks_are_preserved() {
    let body = json!([
        {"id": "A".repeat(19), "name": "too short"},
        {"id": "A".repeat(21), "name": "too long"},
        {"id": "ABCDEFGHIJKLMNOPQRS_", "name": "not alphanumeric"},
        {"id": "AAAAAAAAAAAAAAAAAAé", "name": "not ASCII"},
        {"id": 42, "name": "not a string"},
        {"id": THIRD},
        {"id": FIRST, "name": null},
        {"id": SECOND, "lastOpenedAt": null}
    ]);
    assert_eq!(collect_for("team-B", body), vec![FIRST.to_owned(), SECOND.to_owned()]);
}

#[test]
fn user_and_owner_keys_do_not_supply_or_override_team_identity() {
    let body = json!({"userId": "team-C", "ownerId": "team-C", "projects": [
        {"id": FIRST, "name": "route-bound", "ownerId": "team-C"},
        {"id": SECOND, "name": "matching", "teamId": "team-B", "userId": "team-C"},
        {"id": THIRD, "name": "contradictory", "teamId": "team-C", "ownerId": "team-B"}
    ]});
    assert_eq!(collect_for("team-B", body), vec![FIRST.to_owned(), SECOND.to_owned()]);
}
