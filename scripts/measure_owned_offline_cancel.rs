use std::{fs, path::PathBuf, time::{Duration, Instant}};
use offline_worker_owner::{Cleanup, FilePin, SavedDevice, Specification, Target, Worker};
use serde_json::{json, Value};

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
    value("memory.max") == Some(128 * 1024 * 1024)
        && value("memory.swap.max") == Some(0) && value("pids.max") == Some(32)
}

fn digest(value: &str) -> Result<[u8; 32], &'static str> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("configuration");
    }
    let mut out = [0; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16)
            .map_err(|_| "configuration")?;
    }
    Ok(out)
}

fn configured_worker() -> Result<Worker, &'static str> {
    let bytes = fs::read("/work/specification.json").map_err(|_| "configuration")?;
    if bytes.len() > 65536 { return Err("configuration") }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "configuration")?;
    let object = value.as_object().ok_or("configuration")?;
    if object.len() != 3 { return Err("configuration") }
    let path = |key: &str| -> Result<PathBuf, &'static str> {
        let value = object.get(key).and_then(Value::as_str).ok_or("configuration")?;
        if value.len() > 4096 { return Err("configuration") }
        Ok(PathBuf::from(value))
    };
    let records = object.get("pins").and_then(Value::as_array).ok_or("configuration")?;
    if records.len() > 64 { return Err("configuration") }
    let mut pins = Vec::new();
    for record in records {
        let record = record.as_object().ok_or("configuration")?;
        if record.len() != 2 { return Err("configuration") }
        let name = record.get("path").and_then(Value::as_str).ok_or("configuration")?;
        let hash = record.get("sha256").and_then(Value::as_str).ok_or("configuration")?;
        if name.len() > 4096 { return Err("configuration") }
        pins.push(FilePin { path: PathBuf::from(name), sha256: digest(hash)? });
    }
    Worker::new(Specification { node: path("node")?, entry: path("entry")?, pins })
        .map_err(|_| "configuration")
}

fn child_activity(pid: u32) -> Option<(u64, u64)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, fields) = stat.rsplit_once(')')?;
    let fields: Vec<_> = fields.split_whitespace().collect();
    if matches!(fields.first().copied(), Some("Z" | "X" | "x") | None) {
        return None;
    }
    let user: u64 = fields.get(11)?.parse().ok()?;
    let system: u64 = fields.get(12)?.parse().ok()?;
    let start: u64 = fields.get(19)?.parse().ok()?;
    Some((start, user.checked_add(system)?))
}

async fn measure(worker: Worker) -> Result<Value, &'static str> {
    let mode = std::env::args().nth(1).ok_or("mode")?;
    if !matches!(mode.as_str(), "cancel" | "drop") { return Err("mode") }
    let device = SavedDevice::new("synthetic-owner-cancellation".to_owned())
        .map_err(|_| "device")?;
    let target = Target { project_id: "fixture-project".to_owned(),
        path: "/ai/v3/responses/".to_owned(),
        page_url: "https://framer.com/projects/fixture-project".to_owned() };
    let started = Instant::now();
    let mut run = worker.begin_with_device(target, &device, started + Duration::from_secs(16))
        .map_err(|_| "start")?;
    let completion = run.completion();
    let observed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(pid) = run.started_pid() {
                if let Some((identity, ticks)) = child_activity(pid) {
                    if ticks >= 10 { break (pid, identity, ticks); }
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await;
    let observed = observed.ok().filter(|&(pid, identity, ticks)| {
        child_activity(pid).is_some_and(|(current, total)| {
            current == identity && total >= ticks
        })
    });
    let Some((pid, identity, ticks)) = observed else {
        run.cancel();
        let _ = completion.wait().await;
        drop(run);
        return Err("activity");
    };
    let interrupt_started = Instant::now();
    let rejected = if mode == "cancel" {
        run.cancel();
        match run.result().await {
            Err(offline_worker_owner::Failure::Cancelled) => true,
            Ok(minted) => {
                let mut token = minted.into_token().into_bytes();
                token.fill(0);
                false
            },
            Err(_) => false,
        }
    } else {
        true
    };
    drop(run);
    let cleanup = completion.wait().await;
    let absent = fs::metadata(format!("/proc/{pid}"))
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
    if !rejected || cleanup != (Cleanup::Joined { pid }) || !absent {
        return Err("cleanup");
    }
    Ok(json!({
        "accepted": true, "synthetic": true, "authenticated_delivery": false,
        "cancel_method": mode, "observed_running": true,
        "worker_pid": pid, "worker_start_ticks": identity,
        "observed_cpu_ticks": ticks, "sdk_phase_observed": false,
        "worker_joined": true, "worker_absent": absent,
        "result_check": if mode == "cancel" { "cancelled" } else { "not_requested" },
        "owner_elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        "interrupt_to_join_ms": interrupt_started.elapsed().as_secs_f64() * 1000.0
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
    let accepted = outcome.is_ok();
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
