//! Non-secret, explicit runtime pins. Targets arrive only in each request.
use crate::worker::process::FilePin;
use crate::worker::process::Specification;
use crate::worker::process::Worker;
use serde::Deserialize;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::os::unix::fs::OpenOptionsExt;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config { node: PathBuf, entry: PathBuf, pins: Vec<Pin> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pin { path: PathBuf, sha256: String }

fn digest(value: &str) -> Result<[u8; 32], &'static str> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid offline runtime digest");
    }
    let mut output = [0; 32];
    for (i, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).unwrap();
    }
    Ok(output)
}

pub(crate) fn read(path: &str) -> Result<Vec<u8>, &'static str> {
    let file = std::fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path).map_err(|_| "cannot open offline runtime config")?;
    let stat = file.metadata().map_err(|_| "cannot inspect offline runtime config")?;
    if !stat.is_file() || stat.len() > 64 * 1024 { return Err("invalid offline runtime config"); }
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut bytes)
        .map_err(|_| "cannot read offline runtime config")?;
    if bytes.len() > 64 * 1024 { return Err("offline runtime config too large"); }
    Ok(bytes)
}

pub fn load(path: &str) -> Result<Worker, &'static str> {
    Worker::new(specification(path)?).map_err(|_| "invalid offline worker specification")
}

pub(crate) fn load_v2(path: &str) -> Result<crate::worker::process::V2Worker, &'static str> {
    crate::worker::process::V2Worker::new(specification(path)?)
        .map_err(|_| "invalid V2 worker specification")
}

fn specification(path: &str) -> Result<Specification, &'static str> {
    let bytes = read(path)?;
    let config: Config = serde_json::from_slice(&bytes).map_err(|_| "invalid offline runtime config")?;
    let config_path = Path::new(path).canonicalize().map_err(|_| "runtime config path is unavailable")?;
    let base = config_path.parent().ok_or("runtime config directory is unavailable")?;
    let resolve = |value: PathBuf| {
        if value.is_absolute() { return Ok(value); }
        if value.as_os_str().is_empty()
            || value.components().any(|part| !matches!(part, Component::Normal(_))) {
            return Err("invalid relative runtime path");
        }
        Ok(base.join(value))
    };
    let pins = config.pins.into_iter().map(|pin|
        Ok(FilePin { path: resolve(pin.path)?, sha256: digest(&pin.sha256)? }))
        .collect::<Result<Vec<_>, &'static str>>()?;
    Ok(Specification { node: resolve(config.node)?, entry: resolve(config.entry)?, pins })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("worker-config-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn config(&self, node: &Path, entry: &Path) -> PathBuf {
            let path = self.0.join("worker.json");
            let hash = "a".repeat(64);
            std::fs::write(&path, serde_json::to_vec(&json!({
                "node": node, "entry": entry, "pins": [
                    {"path": node, "sha256": hash}, {"path": entry, "sha256": hash}]
            })).unwrap()).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    #[test]
    fn relative_paths_follow_relocated_config_not_working_directory() {
        let fixture = Fixture::new();
        fixture.config(Path::new("bin/node"), Path::new("worker.mjs"));
        let moved = fixture.0.join("relocated");
        std::fs::create_dir(&moved).unwrap();
        let config = moved.join("worker.json");
        std::fs::rename(fixture.0.join("worker.json"), &config).unwrap();
        let specification = specification(config.to_str().unwrap()).unwrap();
        assert_eq!(specification.node, moved.join("bin/node"));
        assert_eq!(specification.entry, moved.join("worker.mjs"));
        assert_eq!(specification.pins[0].path, specification.node);
        assert!(Worker::new(specification).is_ok());
    }
    #[test]
    fn existing_absolute_config_remains_supported() {
        let fixture = Fixture::new();
        let node = fixture.0.join("node");
        let entry = fixture.0.join("worker.mjs");
        let config = fixture.config(&node, &entry);
        let specification = specification(config.to_str().unwrap()).unwrap();
        assert_eq!(specification.node, node);
        assert_eq!(specification.entry, entry);
    }
    #[test]
    fn relative_paths_cannot_escape_bundle() {
        let fixture = Fixture::new();
        for node in ["", ".", "../node", "bin/../../node"] {
            let config = fixture.config(Path::new(node), Path::new("worker.mjs"));
            assert!(specification(config.to_str().unwrap()).is_err(), "{node}");
        }
    }
}
