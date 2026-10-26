use super::*;
use sha2::{Digest, Sha256};

#[tokio::test]
async fn worker_failure_and_completion_share_only_the_request_diagnostic_id() {
    const CHILD: &str = "FRAMER_TEST_WORKER_DIAGNOSTICS_DONE";
    if std::env::var_os(CHILD).is_none() {
        let done = std::env::temp_dir().join(format!("worker-diagnostic-{}", uuid::Uuid::new_v4()));
        let name = concat!(module_path!(), "::worker_failure_and_completion_share_only_the_request_diagnostic_id");
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name.split_once("::").unwrap().1, "--nocapture", "--test-threads=1"])
            .env(CHILD, &done).kill_on_drop(true).spawn().unwrap();
        let status = tokio::time::timeout(std::time::Duration::from_secs(20), child.wait()).await;
        if status.is_err() { child.kill().await.unwrap(); }
        assert!(status.expect("diagnostic fixture deadline").unwrap().success());
        assert_eq!(std::fs::read(&done).unwrap(), b"completed");
        std::fs::remove_file(done).unwrap();
        return;
    }
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    let root = Directory(std::env::temp_dir().join(format!("native-trace-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&root.0).unwrap();
    framer_core::observability::init(root.0.to_str().unwrap());
    let entry = root.0.join("failed-worker.mjs");
    let code = "import fs from 'node:fs'; fs.readFileSync(3); fs.closeSync(3); fs.closeSync(4); \
        fs.writeFileSync(5, JSON.stringify({version:1,status:'failed',failure_phase:'handoff'})); \
        fs.closeSync(5); process.exit(2);";
    std::fs::write(&entry, code).unwrap();
    let digest = format!("{:x}", Sha256::digest(code.as_bytes()));
    let worker = Arc::new(Worker::new(crate::worker::process::Specification {
        node: crate::test_support::node(), entry: entry.clone(),
        pins: vec![crate::test_support::node_pin(), pin(entry, &digest)],
    }).unwrap());
    let (client, scope) = account();
    let ids = [uuid::Uuid::new_v4(), uuid::Uuid::new_v4()];
    for id in ids {
        let minter = Arc::new(OfflineWorkerMinter::new(scope.clone(), worker.clone())
            .with_diagnostic_id(Some(id)));
        let pending = prepare(&client, &scope, minter, "diagnostic-fixture");
        assert!(matches!(pending.await, Err(PreparationFailure::WorkerLost)));
        crate::service::http::record_completion(&crate::service::serving::Outcome {
            response: None, evidence: None,
            failure: Some(crate::request::types::FailureKind::Authorization),
            cleanup_confirmed: true,
        }, id);
    }
    close(&client, &scope).await;
    let log = std::fs::read_dir(root.0.join(".logs")).unwrap().next().unwrap().unwrap().path();
    let text = std::fs::read_to_string(log).unwrap();
    let records: Vec<serde_json::Value> = text.lines()
        .map(|line| serde_json::from_str(line).unwrap()).collect();
    for id in ids {
        let rows: Vec<_> = records.iter().filter(|row| row["request_id"] == id.to_string()).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["event"], "offline.worker.failed");
        assert_eq!(rows[0]["phase"], "handoff");
        assert_eq!(rows[1]["event"], "offline.complete");
        assert_eq!(rows[1]["completion"]["cleanup_confirmed"], true);
        for row in rows {
            let serialized = row.to_string();
            for private in ["offline-adapter-fixture", "session=", "diagnostic-fixture"] {
                assert!(!serialized.contains(private));
            }
            assert!(row.get("account").is_none());
        }
    }
    std::fs::write(std::env::var_os(CHILD).unwrap(), b"completed").unwrap();
}
