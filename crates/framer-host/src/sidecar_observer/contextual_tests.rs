use super::*;
use framer_core::{ids::ProjectId, port::BrowserAuthorizationRequest};

fn response() -> Value {
    json!({"source":"framer_wrapper_direct", "project":"fixture-project",
        "path":"/ai/v3/responses/", "page_url":"https://framer.com/projects/fixture-project",
        "token":"fixture-token", "cookie_header":"session=fixture-login; current=value"})
}
fn request(project: &ProjectId) -> BrowserAuthorizationRequest<'_> {
    BrowserAuthorizationRequest { project, path: "/ai/v3/responses/",
        cookie_header: "session=fixture-login" }
}

#[test]
fn contextual_wire_preserves_private_material_and_exact_target() {
    let project = ProjectId("fixture-project".into());
    let value = take_context_authorization(response(), request(&project)).unwrap();
    assert_eq!(value.project, project);
    assert_eq!(value.path, "/ai/v3/responses/");
    assert_eq!(value.page_url, "https://framer.com/projects/fixture-project");
    assert_eq!(value.castle.0, "fixture-token");
    assert_eq!(value.cookie_header, "session=fixture-login; current=value");
}

#[test]
fn contextual_wire_rejects_wrong_source_project_or_route() {
    let project = ProjectId("fixture-project".into());
    for (key, wrong) in [("source", "legacy_mint"), ("project", "other-project"),
        ("path", "/ai/v3/messages/")] {
        let mut value = response();
        value[key] = json!(wrong);
        let result = take_context_authorization(value, request(&project));
        assert!(matches!(result, Err(error) if error == "invalid contextual authorization response"));
    }
}

#[test]
fn contextual_wire_bounds_and_redacts_missing_invalid_or_oversized_material() {
    let project = ProjectId("fixture-project".into());
    for (key, limit) in [("page_url", 4096), ("token", 16 * 1024),
        ("cookie_header", 16 * 1024)] {
        for wrong in [Value::Null, json!(4), json!(""), json!("x".repeat(limit + 1))] {
            let mut value = response();
            value[key] = wrong;
            let result = take_context_authorization(value, request(&project));
            assert!(matches!(result, Err(error) if error == "invalid contextual authorization response"));
        }
        let mut value = response();
        value[key] = json!("x".repeat(limit));
        assert!(take_context_authorization(value, request(&project)).is_ok());
    }
}
