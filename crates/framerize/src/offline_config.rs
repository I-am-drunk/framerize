//! Non-secret, explicit runtime pins. Targets arrive only in each request.
use super::offline_worker_owner::{FilePin, Specification, Worker};
use serde::Deserialize;
use std::{io::Read, path::PathBuf};
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

pub(crate) fn load(path: &str) -> Result<Worker, &'static str> {
    Worker::new(specification(path)?).map_err(|_| "invalid offline worker specification")
}

pub(crate) fn load_v2(path: &str) -> Result<super::offline_worker_owner::V2Worker, &'static str> {
    super::offline_worker_owner::V2Worker::new(specification(path)?)
        .map_err(|_| "invalid V2 worker specification")
}

fn specification(path: &str) -> Result<Specification, &'static str> {
    let bytes = read(path)?;
    let config: Config = serde_json::from_slice(&bytes).map_err(|_| "invalid offline runtime config")?;
    let pins = config.pins.into_iter().map(|pin|
        Ok(FilePin { path: pin.path, sha256: digest(&pin.sha256)? }))
        .collect::<Result<Vec<_>, &'static str>>()?;
    Ok(Specification { node: config.node, entry: config.entry, pins })
}
