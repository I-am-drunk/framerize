use super::*;

#[test]
fn explicit_v3_selection_remains_available() {
    let cli = Cli::try_parse_from(["framerize", "daemon", "--offline-sdk", "v3",
        "--offline-worker-config", "/fixture/v3.json"]).unwrap();
    assert!(matches!(cli.cmd, Cmd::Daemon { offline_sdk: Some(OfflineSdk::V3), .. }));
}

#[test]
fn sdk_selection_cannot_share_resident_ownership() {
    assert!(Cli::try_parse_from(["framerize", "daemon", "--offline-sdk", "v2",
        "--offline-worker-config", "/fixture/v2.json", "--resident-control-socket",
        "/fixture/resident.sock"]).is_err());
}

#[test]
fn sdk_selection_requires_worker_and_rejects_unknown_versions() {
    for sdk in ["v2", "v3"] {
        assert!(Cli::try_parse_from(["framerize", "daemon", "--offline-sdk", sdk]).is_err());
    }
    for sdk in ["v1", "v4", "automatic", ""] {
        assert!(Cli::try_parse_from(["framerize", "daemon", "--offline-sdk", sdk,
            "--offline-worker-config", "/fixture/worker.json"]).is_err());
    }
}

#[test]
fn explicit_v2_selection_preserves_ordinary_worker_path() {
    let cli = Cli::try_parse_from(["framerize", "daemon", "--offline-sdk", "v2",
        "--offline-worker-config", "/fixture/v2.json"]).unwrap();
    match cli.cmd {
        Cmd::Daemon { offline_sdk: Some(OfflineSdk::V2),
            offline_worker_config: Some(path), resident_control_socket: None, .. } =>
            assert_eq!(path, "/fixture/v2.json"),
        _ => panic!("explicit ordinary V2 selection was lost"),
    }
}

#[test]
fn ordinary_startup_preserves_implicit_v3() {
    let plain = Cli::try_parse_from(["framerize", "daemon"]).unwrap();
    assert!(matches!(plain.cmd, Cmd::Daemon { offline_sdk: None, .. }));
    let configured = Cli::try_parse_from(["framerize", "daemon",
        "--offline-worker-config", "/fixture/worker.json"]).unwrap();
    assert!(matches!(configured.cmd, Cmd::Daemon { offline_sdk: None, .. }));
}
