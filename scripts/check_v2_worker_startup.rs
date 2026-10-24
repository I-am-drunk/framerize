//! Reject invalid input through the real V2 process owner, without SDK calls.
use offline_worker_owner::{Cleanup, Failure, FilePin, SavedDevice, Specification, Target, V2Worker};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, time::{Duration, Instant}};

fn isolated() -> Result<(), &'static str> {
    for (kind, name) in [("net", "CHECK_PARENT_NETNS"), ("pid", "CHECK_PARENT_PIDNS")] {
        let current = fs::read_link(format!("/proc/self/ns/{kind}")).map_err(|_| "namespace")?;
        let parent = std::env::var(name).map_err(|_| "namespace")?;
        if current.to_string_lossy() == parent { return Err("namespace"); }
    }
    for path in ["/home/tjninja/.framerize", "/run/user/1000/bus"] {
        if PathBuf::from(path).exists() { return Err("private-runtime-visible"); }
    }
    let text = fs::read_to_string("/proc/self/cgroup").map_err(|_| "cgroup")?;
    let relative = text.lines().find_map(|s| s.strip_prefix("0::")).ok_or("cgroup")?;
    let group = PathBuf::from("/sys/fs/cgroup").join(relative.trim_start_matches('/'));
    for (name, wanted) in [("memory.max", 134217728), ("memory.swap.max", 0), ("pids.max", 32)] {
        let actual = fs::read_to_string(group.join(name)).map_err(|_| "limits")?;
        if actual.trim().parse::<u64>().ok() != Some(wanted) { return Err("limits"); }
    }
    Ok(())
}

fn specification(path: &str) -> Result<Specification, &'static str> {
    let bytes = fs::read(path).map_err(|_| "config")?;
    if bytes.len() > 65536 { return Err("config"); }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "config")?;
    if value.as_object().ok_or("config")?.len() != 3 { return Err("config"); }
    let string = |v: &Value| v.as_str().map(str::to_owned).ok_or("config");
    let mut pins = Vec::new();
    for row in value["pins"].as_array().ok_or("config")? {
        if row.as_object().ok_or("config")?.len() != 2 { return Err("config"); }
        let hash = string(&row["sha256"])?;
        if hash.len() != 64 || !hash.is_ascii() { return Err("config"); }
        let mut digest = [0; 32];
        for (i, byte) in digest.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hash[i * 2..i * 2 + 2], 16).map_err(|_| "config")?;
        }
        pins.push(FilePin { path: string(&row["path"])?.into(), sha256: digest });
    }
    Ok(Specification { node: string(&value["node"])?.into(),
        entry: string(&value["entry"])?.into(), pins })
}

async fn check(spec: Specification) -> Result<Value, &'static str> {
    let worker = V2Worker::new(spec).map_err(|_| "config")?;
    // Target validation rejects this before constructing or using the device.
    let device = SavedDevice::new("untrusted-startup-fixture".into()).map_err(|_| "fixture")?;
    let target = Target { project_id: "startup-fixture".into(),
        path: "/not-an-inference-route".into(), page_url: "invalid:".into() };
    let start = Instant::now();
    let mut run = worker.begin_with_device(target, &device, start + Duration::from_secs(12))
        .map_err(|_| "start")?;
    let completion = run.completion();
    let result = run.result().await;
    let cleanup = completion.wait().await;
    drop(run);
    let Cleanup::Joined { pid } = cleanup else { return Err("join"); };
    let Err(Failure::V2WorkerStage(stage)) = result else { return Err("rejection-kind"); };
    if stage.phase() != "input" || stage.operation().is_some() || stage.device().is_some() {
        return Err("rejection-stage");
    }
    let absent = fs::metadata(format!("/proc/{pid}")).is_err_and(|e|
        e.kind() == std::io::ErrorKind::NotFound);
    if !absent { return Err("child-present"); }
    Ok(json!({"accepted": true, "expected_rejection": "input", "worker_joined": true,
        "worker_pid_in_namespace": pid, "worker_absent": true,
        "sdk_operation_entered": false, "token_handoff": false,
        "elapsed_ms": start.elapsed().as_secs_f64() * 1000.0}))
}

fn main() {
    let outcome = (|| {
        isolated()?;
        let path = std::env::args().nth(1).ok_or("config")?;
        let spec = specification(&path)?;
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()
            .map_err(|_| "runtime")?;
        let outcome = runtime.block_on(check(spec));
        runtime.shutdown_timeout(Duration::from_secs(2));
        outcome
    })();
    let accepted = outcome.is_ok();
    let report = outcome.unwrap_or_else(|phase| json!({"accepted": false, "failure": phase}));
    println!("{report}");
    if !accepted { std::process::exit(1); }
}
