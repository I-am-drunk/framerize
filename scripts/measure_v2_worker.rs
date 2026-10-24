//! Synthetic, socket-denied V2 worker qualification. No account or HTTP.
use offline_worker_owner::{Cleanup, Failure, FilePin, SavedDevice, Specification, Target, V2Worker};
use std::{path::PathBuf, time::{Duration, Instant}};
use serde_json::{json, Value};

fn specification(path: &str) -> Result<Specification, &'static str> {
    let bytes = std::fs::read(path).map_err(|_| "config")?;
    if bytes.len() > 65536 { return Err("config"); }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "config")?;
    let string = |v: &Value| v.as_str().map(str::to_owned).ok_or("config");
    let mut pins = Vec::new();
    for pin in value["pins"].as_array().ok_or("config")? {
        let hex = string(&pin["sha256"])?;
        if hex.len() != 64 || !hex.is_ascii() { return Err("config"); }
        let mut digest = [0; 32];
        for (i, b) in digest.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[i*2..i*2+2], 16).map_err(|_| "config")?;
        }
        pins.push(FilePin { path: PathBuf::from(string(&pin["path"])?), sha256: digest });
    }
    Ok(Specification { node: string(&value["node"])?.into(),
        entry: string(&value["entry"])?.into(), pins })
}

async fn run(path: &str) -> Result<Value, &'static str> {
    let worker = V2Worker::new(specification(path)?).map_err(|_| "config")?;
    let device = SavedDevice::new("00000000000040008000000000000001".into()).map_err(|_| "device")?;
    let target = Target { project_id: "offline-fixture".into(),
        path: "/ai/v3/responses/".into(), page_url: "https://framer.com/projects/offline-fixture".into() };
    let mut run = worker.begin_with_device(target, &device,
        Instant::now() + Duration::from_secs(12)).map_err(|_| "start")?;
    let completion = run.completion();
    let result = run.result().await;
    let joined = matches!(completion.wait().await, Cleanup::Joined { .. });
    match result {
        Ok(minted) => {
            let age = minted.freshness_origin.elapsed().as_secs_f64() * 1000.0;
            let ordered = minted.freshness_origin <= minted.received_at
                && minted.received_at <= Instant::now();
            let facts = minted.metadata;
            let mut token = minted.into_token().into_bytes();
            let bounded = !token.is_empty() && token.len() <= 16384;
            token.fill(0);
            Ok(json!({"passed":joined && ordered && bounded,"cleanup_confirmed":joined,
                "token_discarded":true,"failure":null,"method_ms":facts.operation.method_ms,
                "component_ms":facts.operation.elapsed_ms,"diagnostics":facts.operation.diagnostics,
                "handoff_ms":facts.mint_to_handoff_ms,"conservative_age_ms":age}))
        }
        Err(error) => {
            let (phase, operation, device) = match error {
                Failure::V2WorkerStage(stage) => (stage.phase(), stage.operation(), stage.device()),
                Failure::Deadline => ("deadline", None, None),
                Failure::Cancelled => ("cancelled", None, None),
                Failure::Protocol => ("protocol", None, None),
                _ => ("worker", None, None),
            };
            Ok(json!({"passed":false,"cleanup_confirmed":joined,"token_discarded":false,
                "failure":phase,"operation":operation,"device":device}))
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let result = match std::env::args().nth(1) {
        Some(path) => run(&path).await.unwrap_or_else(|code| json!({"passed":false,"failure":code})),
        None => json!({"passed":false,"failure":"config"}),
    };
    println!("{}", result);
    if result["passed"] != true { std::process::exit(1); }
}
