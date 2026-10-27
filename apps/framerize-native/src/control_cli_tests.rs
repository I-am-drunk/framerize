use super::*;

fn serve(extra: &[&str]) -> Result<Cli, clap::Error> {
    let mut args = vec!["framerize-native", "serve", "--db", "/fixture/db",
        "--profiles", "/fixture/profile", "--accounts", "fixture-account"];
    args.extend_from_slice(extra);
    Cli::try_parse_from(args)
}

#[test]
fn source_choice_does_not_change_sender_choice() {
    for source in [false, true] {
        for sender in [false, true] {
            let mut args = Vec::new();
            if source { args.extend(["--browser-control-descriptor", "/file", "--browser-control-sha256", "digest"]); }
            if sender { args.extend(["--browser-transport-script", "/sender"]); }
            let Command::Serve { browser_control_descriptor, browser_control_sha256,
                browser_transport_script, .. } = serve(&args).unwrap().command else { panic!() };
            assert_eq!(browser_control_descriptor.is_some(), source);
            assert_eq!(browser_control_sha256.is_some(), source);
            assert_eq!(browser_transport_script.is_some(), sender);
        }
    }
}

#[tokio::test]
async fn invalid_descriptor_refuses_before_runtime_or_profile_access() {
    let error = run("/absent/database".into(), "/absent/profile".into(), None,
        vec!["invalid-account".into()], "127.0.0.1:0".parse().unwrap(), "/absent/python".into(),
        None, None, None, Some("/absent/control.json".into()), Some("a".repeat(64))).await;
    assert_eq!(error.err().as_deref(), Some("invalid browser control launch descriptor"));
}

#[tokio::test]
async fn internal_call_also_refuses_partial_control_selection() {
    let error = run("/absent/database".into(), "/absent/profile".into(), None,
        vec![], "127.0.0.1:0".parse().unwrap(), "/absent/python".into(), None, None, None,
        Some("/absent/control.json".into()), None).await;
    assert_eq!(error.err().as_deref(), Some("browser control requires both descriptor and SHA256"));
}

#[test]
fn serve_defaults_keep_native_source_and_native_sender() {
    let Command::Serve { browser_control_descriptor, browser_control_sha256,
        browser_transport_script, .. } = serve(&[]).unwrap().command else { panic!() };
    assert!(browser_control_descriptor.is_none());
    assert!(browser_control_sha256.is_none());
    assert!(browser_transport_script.is_none());
}

#[test]
fn browser_source_requires_both_file_and_digest() {
    assert!(serve(&["--browser-control-descriptor", "/fixture/control.json"]).is_err());
    assert!(serve(&["--browser-control-sha256", "fixture-digest"]).is_err());
}

#[test]
fn descriptor_check_is_separate_from_serving_and_requires_both_inputs() {
    assert!(Cli::try_parse_from(["native", "check-browser-control", "--descriptor", "/file"]).is_err());
    let cli = Cli::try_parse_from(["native", "check-browser-control", "--descriptor", "/file",
        "--sha256", "digest"]).unwrap();
    assert!(matches!(cli.command, Command::CheckBrowserControl { .. }));
}
