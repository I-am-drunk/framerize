//! Local synthetic workers use the Node selected by the developer or CI.
use std::path::PathBuf;
use sha2::{Digest, Sha256};

pub(crate) fn node() -> PathBuf {
    if let Some(path) = std::env::var_os("FRAMERIZE_TEST_NODE") {
        return std::fs::canonicalize(path).expect("FRAMERIZE_TEST_NODE must exist");
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|path| path.join("node"))
        .find(|path| path.is_file())
        .and_then(|path| path.canonicalize().ok())
        .expect("install Node or set FRAMERIZE_TEST_NODE")
}

pub(crate) fn node_pin() -> crate::worker::process::FilePin {
    let path = node();
    let sha256 = Sha256::digest(std::fs::read(&path).unwrap()).into();
    crate::worker::process::FilePin { path, sha256 }
}

pub(crate) fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference/fixtures/offline_v3")
        .canonicalize().unwrap()
}
