use super::*;

#[test]
fn resident_v2_requires_resident_socket_and_cannot_select_serving_worker() {
    assert!(Cli::try_parse_from(["framerize", "daemon",
        "--resident-v2-worker-config", "/fixture/config"]).is_err());
    assert!(Cli::try_parse_from(["framerize", "daemon", "--resident-control-socket",
        "/fixture/socket", "--resident-v2-worker-config", "/fixture/config"]).is_ok());
}

#[test]
fn ordinary_startup_has_no_resident_control_socket() {
    let cli = Cli::try_parse_from(["framerize", "daemon"]).unwrap();
    assert!(matches!(cli.cmd, Cmd::Daemon { resident_control_socket: None, .. }));
}

#[test]
fn resident_control_requires_an_explicit_socket_path() {
    assert!(Cli::try_parse_from(["framerize", "daemon", "--resident-control-socket"]).is_err());
    let cli = Cli::try_parse_from(["framerize", "daemon", "--engine", "camoufox",
        "--resident-control-socket", "/fixture-private/control.sock"]).unwrap();
    match cli.cmd {
        Cmd::Daemon { resident_control_socket: Some(path), .. } =>
            assert_eq!(path, std::path::PathBuf::from("/fixture-private/control.sock")),
        _ => panic!("missing explicit resident control"),
    }
}

#[test]
fn resident_control_cannot_share_an_offline_session_owner() {
    for flag in ["--offline-worker-config", "--offline-routing-config"] {
        assert!(Cli::try_parse_from(["framerize", "daemon", "--resident-control-socket",
            "/fixture-private/control.sock", flag, "/fixture-config"]).is_err());
    }
}
