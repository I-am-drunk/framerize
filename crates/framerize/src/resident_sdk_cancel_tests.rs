use super::*;
use std::{path::PathBuf, time::Duration};
use offline_worker_owner::{FilePin, Run, Specification};

fn pin(path: PathBuf, hex: &str) -> FilePin {
    let sha256 = std::array::from_fn(|i|
        u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap());
    FilePin { path, sha256 }
}

struct Fixture { root: PathBuf, worker: V2Worker }
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("resident-sdk-cancel-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let entry = root.join("worker.mjs");
        std::fs::write(&entry, include_bytes!("../../../reference/fixtures/offline_v2_worker_fake.mjs")).unwrap();
        let node = PathBuf::from("/home/tjninja/.nvm/versions/node/v24.13.0/bin/node");
        let pins = vec![
            pin(node.clone(), "53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc"),
            pin(entry.clone(), "e3a3e8e7fd928778b8d7f5087b78deec63662d5c0f71c7d521f47997d41bd2ce"),
        ];
        Self { root, worker: V2Worker::new(Specification { node, entry, pins }).unwrap() }
    }
    fn begin(&self) -> Run<V2Metadata> {
        self.worker.begin_with_device(Target { project_id: "hang".into(),
            path: "/ai/v3/responses/".into(), page_url: "https://framer.com/projects/hang".into() },
            &SavedDevice::new("fixture-device".into()).unwrap(),
            std::time::Instant::now() + Duration::from_secs(12)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

#[tokio::test(flavor = "current_thread")]
async fn ready_cancel_before_result_poll_starts_no_child() {
    let fixture = Fixture::new();
    let mut run = fixture.begin();
    let completion = run.completion();
    let result = wait_for_result(&mut run, std::future::ready(())).await;
    let observed = tokio::time::timeout(Duration::from_secs(2), completion.clone().wait()).await;
    run.cancel();
    let drained = completion.wait().await;
    assert!(matches!(result, Err(Failure::Cancelled)));
    assert_eq!(observed.unwrap(), Cleanup::NotStarted);
    assert_eq!(drained, Cleanup::NotStarted);
    assert_eq!(run.started_pid(), None);
    assert!(matches!(run.result().await, Err(offline_worker_owner::Failure::Cancelled)));
}

#[tokio::test(flavor = "current_thread")]
async fn ready_cancel_before_result_poll_joins_started_child() {
    let fixture = Fixture::new();
    let mut run = fixture.begin();
    let completion = run.completion();
    let pid = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(pid) = run.started_pid() { break pid; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap();
    let result = wait_for_result(&mut run, std::future::ready(())).await;
    let observed = tokio::time::timeout(Duration::from_secs(2), completion.clone().wait()).await;
    run.cancel();
    let drained = completion.wait().await;
    assert!(matches!(result, Err(Failure::Cancelled)));
    assert_eq!(observed.unwrap(), Cleanup::Joined { pid });
    assert_eq!(drained, Cleanup::Joined { pid });
    assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
    assert!(matches!(run.result().await, Err(offline_worker_owner::Failure::Cancelled)));
}
