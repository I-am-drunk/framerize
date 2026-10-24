use super::*;
use framer_core::ids::FramerUserId;

const PROJECT: &str = "AB12cd34EF56gh78IJ90";
const LOGIN: &str = "session=fixture-login";

#[test]
fn saved_device_preserves_the_login_and_existing_cookie_bytes() {
    for (header, expected) in [
        (LOGIN, "session=fixture-login; __cuid=saved-device"),
        ("theme=dark; __cuid=saved-device; session=fixture-login",
         "theme=dark; __cuid=saved-device; session=fixture-login"),
    ] {
        let seed = session(header);
        let cookies = CurrentRequestCookies::from_saved_device(&seed, "saved-device").unwrap();
        let current = cookies.into_session(&seed).unwrap();
        assert_eq!(current.cookie_header, expected);
        assert_eq!(current.active_user, seed.active_user);
        assert_eq!(seed.cookie_header, header);
    }
}

#[test]
fn saved_device_rejects_conflicts_injection_and_header_overflow() {
    let oversized = "x".repeat(257);
    for device in ["", "x;y", "x=y", "with space", "é", "x\r\ny", &oversized] {
        assert!(CurrentRequestCookies::from_saved_device(&session(LOGIN), device).is_err());
    }
    for header in [
        "session=fixture-login; __cuid=other-device",
        "session=fixture-login; __cuid=saved-device; __cuid=saved-device",
        "session=other; session=fixture-login",
    ] {
        assert!(CurrentRequestCookies::from_saved_device(&session(header), "saved-device").is_err());
    }
    let prefix = "session=fixture-login; padding=";
    let full = format!("{prefix}{}", "x".repeat(16 * 1024 - prefix.len()));
    assert!(CurrentRequestCookies::from_saved_device(&session(&full), "saved-device").is_err());
}

fn session(header: &str) -> Session {
    Session {
        active_user: FramerUserId("fixture-private-user".into()),
        cookie_header: header.into(),
    }
}

fn target(project: &str, path: &'static str) -> CredentialTarget {
    CredentialTarget { project: ProjectId(project.into()), path }
}

#[test]
fn current_jar_preserves_exact_bytes_and_private_user() {
    let seed = session(LOGIN);
    let header = "theme=dark; session=fixture-login; context=fresh%2Bvalue; empty=";
    let jar = CurrentRequestCookies::from_current_jar(&seed, header.into())
        .expect("valid current jar");
    let result = jar.into_session(&seed.clone()).expect("same private session");
    assert_eq!(result.cookie_header, header);
    assert_eq!(result.active_user, seed.active_user);
    assert_eq!(seed.cookie_header, LOGIN);
}

#[test]
fn current_jar_cannot_cross_private_user_or_seed_header() {
    let seed = session(LOGIN);
    let mut other_user = seed.clone();
    other_user.active_user = FramerUserId("fixture-other-user".into());
    let other_header = session("session=fixture-login; context=different-seed");
    for expected in [other_user, other_header] {
        let jar = CurrentRequestCookies::from_current_jar(
            &seed, "session=fixture-login; context=fresh".into(),
        ).expect("valid current jar");
        assert!(jar.into_session(&expected).is_err());
    }
}

#[test]
fn changed_login_is_rejected_even_with_unchanged_other_cookies() {
    let seed = session("session=fixture-login; context=same");
    for header in [
        "session=fixture-other; context=same",
        "session=fixture-login-extra; context=same",
        "session=fixture-login%20; context=same",
    ] {
        assert!(CurrentRequestCookies::from_current_jar(&seed, header.into()).is_err());
    }
}

#[test]
fn missing_empty_or_duplicate_login_is_rejected_in_both_jars() {
    let seed = session(LOGIN);
    for header in [
        "",
        "theme=dark",
        "session=",
        "Session=fixture-login",
        "session=fixture-login; session=fixture-login",
        "session=fixture-login; session=fixture-other",
    ] {
        assert!(CurrentRequestCookies::from_current_jar(&seed, header.into()).is_err());
        assert!(CurrentRequestCookies::from_current_jar(
            &session(header), LOGIN.into(),
        ).is_err());
    }
}

#[test]
fn malformed_cookie_syntax_and_header_injection_are_rejected() {
    let seed = session(LOGIN);
    for header in [
        "session=fixture-login\r\nX-Injected: 1",
        "session=fixture-login; context=a\nb",
        "session=fixture-login; context=a\tb",
        "session=fixture-login; context=a\0b",
        "session=fixture-login; context=a\x7fb",
        "session=fixture-login; context=é",
        "session=fixture-login; context=a b",
        "session=fixture-login; context=a,b",
        r#"session=fixture-login; context="quoted""#,
        r"session=fixture-login; context=a\b",
        "session=fixture-login; bad:name=value",
        "session=fixture-login; orphan",
        "session=fixture-login; =value",
    ] {
        assert!(CurrentRequestCookies::from_current_jar(&seed, header.into()).is_err());
        assert!(CurrentRequestCookies::from_current_jar(
            &session(header), LOGIN.into(),
        ).is_err());
    }
}

#[test]
fn cookie_header_bound_applies_to_seed_and_current_snapshot() {
    let seed = session(LOGIN);
    let prefix = "session=fixture-login; context=";
    let maximum = format!("{prefix}{}", "x".repeat(16 * 1024 - prefix.len()));
    assert_eq!(maximum.len(), 16 * 1024);
    let jar = CurrentRequestCookies::from_current_jar(&seed, maximum.clone())
        .expect("inclusive current-header bound");
    assert_eq!(jar.into_session(&seed).expect("same seed").cookie_header, maximum);
    assert!(CurrentRequestCookies::from_current_jar(
        &session(&maximum), LOGIN.into(),
    ).is_ok());
    let oversized = format!("{maximum}x");
    assert!(CurrentRequestCookies::from_current_jar(&seed, oversized.clone()).is_err());
    assert!(CurrentRequestCookies::from_current_jar(
        &session(&oversized), LOGIN.into(),
    ).is_err());
}

#[test]
fn browser_target_accepts_only_supported_routes_and_bounded_ids() {
    let longest = "A".repeat(256);
    for path in ["/ai/v3/responses/", "/ai/v3/messages/"] {
        for project in [PROJECT, "a", "fixture-project_1", longest.as_str()] {
            assert!(target(project, path).valid_for_browser());
        }
    }
}

#[test]
fn browser_target_rejects_ambiguous_ids_and_other_routes() {
    let oversized = "A".repeat(257);
    for project in ["", "bad/project", "bad space", "bad?query", "bad#fragment",
                    "é", oversized.as_str()] {
        assert!(!target(project, "/ai/v3/responses/").valid_for_browser());
    }
    for path in ["/ai/v3/openai/responses/", "/ai/v3/responses",
                 "/ai/v3/responses/?query=1", "/auth/ai/access-token",
                 "https://api.framer.com/ai/v3/responses/"] {
        assert!(!target(PROJECT, path).valid_for_browser());
    }
}

#[test]
fn project_page_requires_exact_id_or_exact_slug_suffix() {
    let target = target(PROJECT, "/ai/v3/responses/");
    for page in [
        format!("https://framer.com/projects/{PROJECT}"),
        format!("https://framer.com/projects/demo--{PROJECT}"),
        format!("https://framer.com/projects/demo--{PROJECT}/?node=fixture#inspect"),
    ] {
        assert!(target.matches_page(&page));
    }
    for page in [
        format!("https://framer.com/projects/prefix{PROJECT}"),
        format!("https://framer.com/projects/demo--{PROJECT}suffix"),
        format!("https://framer.com/projects/wrong?project={PROJECT}"),
        format!("https://framer.com/projects/wrong#{PROJECT}"),
        format!("https://framer.com/projects/{PROJECT}/child"),
        format!("https://framer.com/prefix/projects/{PROJECT}"),
    ] {
        assert!(!target.matches_page(&page));
    }
}

#[test]
fn project_page_rejects_wrong_origin_and_userinfo() {
    let target = target(PROJECT, "/ai/v3/messages/");
    for origin in [
        "http://framer.com",
        "https://example.test",
        "https://framer.com.example.test",
        "https://app.framer.com",
        "https://framer.com:8443",
        "https://fixture@framer.com",
        "https://fixture:password@framer.com",
        "file://framer.com",
    ] {
        assert!(!target.matches_page(&format!("{origin}/projects/{PROJECT}")));
    }
    assert!(!target.matches_page(&format!("/projects/{PROJECT}")));
    assert!(!target.matches_page("not a URL"));
}

#[test]
fn project_access_links_preserve_exact_identity() {
    let target = target(PROJECT, "/ai/v3/responses/");
    for stem in [PROJECT.to_owned(), format!("My-Site--{PROJECT}")] {
        assert!(target.matches_page(&format!(
            "https://framer.com/projects/{stem}-fixtureAccess1?node=fixture")));
    }
    for path in [format!("{PROJECT}-"), format!("{PROJECT}-bad_suffix"),
        format!("{PROJECT}-bad-suffix"), format!("{PROJECT}-fixture/child"),
        format!("{PROJECT}%2Dfixture"), format!("Bad_Title--{PROJECT}-fixture"),
        format!("{PROJECT}--ZZ12cd34EF56gh78IJ90-fixture"),
        "My-Site--ZZ12cd34EF56gh78IJ90-fixture".into()] {
        assert!(!target.matches_page(&format!("https://framer.com/projects/{path}")));
    }
}

#[test]
fn page_validation_retains_bounds_and_supported_target_requirement() {
    let valid = target(PROJECT, "/ai/v3/responses/");
    let too_long = format!("https://framer.com/projects/{PROJECT}?{}", "x".repeat(4096));
    assert!(!valid.matches_page(&too_long));
    let unsupported = target(PROJECT, "/ai/v3/openai/responses/");
    assert!(!unsupported.matches_page(&format!("https://framer.com/projects/{PROJECT}")));
}

#[test]
fn routing_offline_target_accepts_registered_inference_routes_and_exact_pages() {
    use crate::experimental_native::control_authorization::OfflineMintTarget;
    let mut paths = Vec::new();
    for route in crate::routes::inference() {
        paths.push(route.path);
        for page in [format!("https://framer.com/projects/{PROJECT}"),
            format!("https://framer.com/projects/demo--{PROJECT}/?node=fixture")] {
            let bound = OfflineMintTarget::new(target(PROJECT, route.path), page.clone()).unwrap();
            assert_eq!(bound.path(), route.path);
            assert_eq!(bound.project().0, PROJECT);
            assert_eq!(bound.page_url(), page);
        }
    }
    paths.sort_unstable();
    assert_eq!(paths, ["/ai/v3/anthropic/messages/", "/ai/v3/chat/", "/ai/v3/messages/",
        "/ai/v3/openai/responses/", "/ai/v3/responses/"]);
}

#[test]
fn routing_offline_target_rejects_wrong_scope_and_non_inference_routes() {
    use crate::experimental_native::control_authorization::OfflineMintTarget;
    for route in crate::routes::inference() {
        for page in ["http://framer.com/projects/", "https://example.test/projects/",
            "https://fixture@framer.com/projects/", "https://framer.com:8443/projects/",
            "https://framer.com/projects/wrong?project="] {
            assert!(OfflineMintTarget::new(target(PROJECT, route.path),
                format!("{page}{PROJECT}")).is_err());
        }
        for project in ["", "bad/project", "bad space", "bad?query", "é"] {
            assert!(OfflineMintTarget::new(target(project, route.path),
                format!("https://framer.com/projects/{project}")).is_err());
        }
    }
    for path in ["/ai/v3/responses", "/ai/v3/responses/?query=1",
        "/auth/ai/access-token", "https://api.framer.com/ai/v3/responses/"] {
        assert!(OfflineMintTarget::new(target(PROJECT, path),
            format!("https://framer.com/projects/{PROJECT}")).is_err());
    }
    for route in [&crate::routes::refund::ROUTE, &crate::routes::fetch::ROUTE] {
        assert!(OfflineMintTarget::new(target(PROJECT, route.path),
            format!("https://framer.com/projects/{PROJECT}")).is_err());
    }
}
