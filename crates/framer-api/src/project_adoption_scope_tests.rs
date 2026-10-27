//! Inert first-project selection coverage; no transport or creator is invoked.
use super::project_to_adopt;
use framer_core::ids::{ProjectId, TeamId};
use serde_json::json;

const TEAM: &str = "01234567-89ab-4cde-8fab-012345678901";
const OTHER: &str = "01234567-89ab-4cde-8fab-012345678902";
const FIRST: &str = "0123456789ABCDEFGHIJ";
const SECOND: &str = "abcdefghij0123456789";
const ERROR: &str = "project adoption response has incompatible teamId";

#[test]
fn missing_first_project_id_keeps_no_adoption() {
    for body in [json!(null), json!([]), json!({}), json!({"teamId": OTHER}),
        json!({"teamId": null, "projects": null}),
        json!({"teamId": OTHER, "projects": {"id": FIRST}}),
        json!({"teamId": OTHER, "projects": "not a list"}),
        json!({"teamId": OTHER, "projects": []})] {
        assert_eq!(project_to_adopt(&body, &TeamId(TEAM.into())), Ok(None));
    }
    for first in [json!(null), json!(false), json!(7), json!(FIRST), json!([]),
        json!({}), json!({"id": null, "teamId": null}),
        json!({"id": 7, "teamId": OTHER}), json!({"id": false}),
        json!({"id": []}), json!({"id": {}})] {
        let body = json!({"teamId": OTHER, "projects": [
            first, {"id": SECOND, "teamId": TEAM}
        ]});
        assert_eq!(project_to_adopt(&body, &TeamId(TEAM.into())), Ok(None));
    }
}

#[test]
fn matching_or_absent_team_keeps_first_project() {
    for id in [FIRST, "", " \t ", "unicode-雪"] {
        for root_team in [false, true] {
            for project_team in [false, true] {
                let mut body = json!({"userId": OTHER, "ownerId": OTHER, "projects": [
                    {"id": id, "userId": OTHER, "ownerId": OTHER},
                    {"id": SECOND, "teamId": OTHER}
                ]});
                if root_team { body["teamId"] = json!(TEAM); }
                if project_team { body["projects"][0]["teamId"] = json!(TEAM); }
                assert_eq!(project_to_adopt(&body, &TeamId(TEAM.into())),
                    Ok(Some(ProjectId(id.into()))));
            }
        }
    }
}

#[test]
fn contradictory_response_team_is_rejected() {
    for first in [json!({"id": FIRST}), json!({"id": FIRST, "teamId": TEAM})] {
        let body = json!({"teamId": OTHER, "projects": [first, {"id": SECOND}]});
        assert_eq!(project_to_adopt(&body, &TeamId(TEAM.into())), Err(ERROR.into()));
    }
}

#[test]
fn malformed_team_metadata_is_rejected() {
    for invalid in [json!(null), json!(false), json!(7), json!(1.5), json!([]),
        json!({}), json!(""), json!(format!(" {TEAM} "))] {
        for at_root in [true, false] {
            let mut body = json!({"teamId": TEAM, "projects": [{"id": FIRST, "teamId": TEAM}]});
            if at_root {
                body["teamId"] = invalid.clone();
            } else {
                body["projects"][0]["teamId"] = invalid.clone();
            }
            assert_eq!(project_to_adopt(&body, &TeamId(TEAM.into())), Err(ERROR.into()));
        }
    }
}

#[test]
fn foreign_selected_project_team_is_rejected() {
    for foreign in [OTHER, "01234567", "01234567-89AB-4CDE-8FAB-012345678901"] {
        let body = json!({"teamId": TEAM, "projects": [
            {"id": FIRST, "teamId": foreign}, {"id": SECOND, "teamId": TEAM}
        ]});
        assert_eq!(project_to_adopt(&body, &TeamId(TEAM.into())), Err(ERROR.into()));
    }
}
