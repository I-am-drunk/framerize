use std::{fs, path::PathBuf, time::{Duration, Instant}};
use framer_native::worker::process::{Cleanup, SavedDevice, Target, Worker};
use serde_json::{json, Value};

const GROUP_LIMIT: u64 = 128 * 1024 * 1024;

fn isolated() -> bool {
    let namespace = |kind: &str| fs::read_link(format!("/proc/self/ns/{kind}"))
        .ok().map(|p| p.to_string_lossy().into_owned());
    for (kind, variable) in [
        ("net", "FRAMERIZE_PARENT_NETNS"), ("pid", "FRAMERIZE_PARENT_PIDNS"),
    ] {
        let Ok(parent) = std::env::var(variable) else { return false };
        if namespace(kind).as_deref() == Some(parent.as_str()) { return false }
        if namespace(kind).is_none() { return false }
    }
    if PathBuf::from("/home/tjninja/.framerize").exists()
        || PathBuf::from("/run/user/1000/bus").exists() { return false }
    let Ok(group) = fs::read_to_string("/proc/self/cgroup") else { return false };
    let Some((_, group)) = group.trim().split_once("::") else { return false };
    let root = PathBuf::from("/sys/fs/cgroup").join(group.trim_start_matches('/'));
    let value = |name| fs::read_to_string(root.join(name)).ok()
        .and_then(|s| s.trim().parse::<u64>().ok());
    value("memory.max") == Some(GROUP_LIMIT)
        && value("memory.swap.max") == Some(0) && value("pids.max") == Some(32)
}

fn configured_worker() -> Result<Worker, &'static str> {
    framer_native::worker::from_config("/work/worker-config.json")
}

async fn measure(worker: Worker) -> Result<Value, &'static str> {
    let device = SavedDevice::new("synthetic-owner-measurement".to_owned())
        .map_err(|_| "device")?;
    let target = Target { project_id: "fixture-project".to_owned(),
        path: "/ai/v3/responses/".to_owned(),
        page_url: "https://framer.com/projects/fixture-project".to_owned() };
    let started = Instant::now();
    let mut run = worker.begin_with_device(target, &device, started + Duration::from_secs(16))
        .map_err(|_| "start")?;
    let completion = run.completion();
    let minted = run.result().await;
    let cleanup = completion.wait().await;
    drop(run);
    let Cleanup::Joined { pid } = cleanup else { return Err("cleanup") };
    let absent = fs::metadata(format!("/proc/{pid}"))
        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
    let minted = match minted {
        Ok(value) => value,
        Err(failure) => return Ok(json!({
            "accepted": false, "synthetic": true, "authenticated_delivery": false,
            "worker_joined": true, "worker_absent": absent,
            "failure": format!("{failure:?}")
        })),
    };
    if minted.freshness_origin > minted.received_at || minted.received_at > Instant::now() {
        return Err("freshness");
    }
    let metadata = minted.metadata.clone();
    let mut bytes = minted.into_token().into_bytes();
    let valid = !bytes.is_empty() && bytes.len() <= 16384;
    bytes.fill(0);
    drop(bytes);
    if !valid || !absent { return Err("handoff") }
    Ok(json!({
        "accepted": true, "synthetic": true, "authenticated_delivery": false,
        "worker_joined": true, "worker_pid": pid, "worker_absent": absent,
        "token_discarded": valid,
        "owner_elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        "sdk_elapsed_ms": metadata.elapsed_ms, "sdk_method_ms": metadata.method_ms,
        "handoff_ms": metadata.collection_to_handoff_ms
    }))
}

fn main() {
    use std::io::Write;
    if !isolated() { std::process::exit(2) }
    let outcome = (|| -> Result<Value, &'static str> {
        let worker = configured_worker()?;
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all()
            .build().map_err(|_| "runtime")?;
        let result = runtime.block_on(measure(worker));
        runtime.shutdown_timeout(Duration::from_secs(2));
        result
    })();
    let accepted = outcome.as_ref().is_ok_and(|value| value["accepted"] == true);
    let report = outcome.unwrap_or_else(|phase| json!({
        "accepted": false, "synthetic": true, "failure_phase": phase
    }));
    let write = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut output = fs::OpenOptions::new().write(true).create_new(true)
            .open("/out/result.json")?;
        output.write_all(&serde_json::to_vec(&report)?)?;
        output.write_all(b"\n")?;
        Ok(())
    })();
    if write.is_err() { std::process::exit(3) }
    std::process::exit(if accepted { 0 } else { 2 });
}
