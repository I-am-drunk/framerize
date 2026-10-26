//! Temporary files and controlled blocking writers only; no browser or account.
use super::*;
use crate::DeviceRecord;
use std::{sync::mpsc, time::Duration};
use tokio::{sync::oneshot, time::timeout};

const WINNER: &str = "{ \"castle_device_id\": \"winner\", \"future\": [1, true] }\n";
const LOSER: &[u8] = b"{\"castle_device_id\":\"loser\"}";

struct OwnerDone(Option<oneshot::Sender<()>>);
impl Drop for OwnerDone {
    fn drop(&mut self) { if let Some(done) = self.0.take() { let _ = done.send(()); } }
}

struct Pending {
    task: tokio::task::JoinHandle<io::Result<String>>,
    ready: oneshot::Receiver<()>,
    release: mpsc::Sender<()>,
    finished: oneshot::Receiver<()>,
}

fn pending(path: PathBuf, contents: Vec<u8>) -> Pending {
    let (ready, ready_rx) = oneshot::channel();
    let (release, release_rx) = mpsc::channel();
    let (done, finished) = oneshot::channel();
    let owner_done = OwnerDone(Some(done));
    let mut ready = Some(ready);
    let task = tokio::spawn(create_with(path, contents, move |file, bytes| {
        let _keep_owner_until_publication_finishes = &owner_done;
        let split = bytes.len() / 2;
        file.write_all(&bytes[..split])?;
        let _ = ready.take().unwrap().send(());
        release_rx.recv_timeout(Duration::from_secs(3))
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "fixture release missing"))?;
        file.write_all(&bytes[split..])
    }));
    Pending { task, ready: ready_rx, release, finished }
}

struct Profile(PathBuf);

impl Profile {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("cdp-publish-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn record(&self) -> PathBuf { DeviceRecord::path(&self.0) }
    fn entries(&self) -> usize { std::fs::read_dir(&self.0).unwrap().count() }
}

impl Drop for Profile {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

#[tokio::test]
async fn competing_candidates_return_the_first_published_record_unchanged() {
    let profile = Profile::new();
    let mut first = pending(profile.record(), WINNER.as_bytes().to_vec());
    let mut later = pending(profile.record(), LOSER.to_vec());
    let first_ready = timeout(Duration::from_secs(3), &mut first.ready).await;
    let later_ready = timeout(Duration::from_secs(3), &mut later.ready).await;
    let unpublished = !profile.record().exists();
    let candidates = profile.entries();
    let _ = first.release.send(());
    let first_result = timeout(Duration::from_secs(3), &mut first.task).await;
    let _ = later.release.send(());
    let later_result = timeout(Duration::from_secs(3), &mut later.task).await;
    let first_done = timeout(Duration::from_secs(3), first.finished).await;
    let later_done = timeout(Duration::from_secs(3), later.finished).await;
    first_ready.unwrap().unwrap();
    later_ready.unwrap().unwrap();
    first_done.unwrap().unwrap();
    later_done.unwrap().unwrap();
    assert!(unpublished, "an incomplete candidate was visible as the record");
    assert_eq!(candidates, 2);
    assert_eq!(first_result.unwrap().unwrap().unwrap(), WINNER);
    assert_eq!(later_result.unwrap().unwrap().unwrap(), WINNER);
    assert_eq!(std::fs::read_to_string(profile.record()).unwrap(), WINNER);
    assert_eq!(DeviceRecord::load_or_create(&profile.0).await.unwrap(), "winner");
    assert_eq!(profile.entries(), 1, "private candidates survived publication");
}

#[tokio::test]
async fn partial_candidate_failure_leaves_no_record_and_allows_retry() {
    let profile = Profile::new();
    let result = create_with(profile.record(), LOSER.to_vec(), |file, bytes| {
        file.write_all(&bytes[..8])?;
        Err(io::Error::new(io::ErrorKind::Other, "fixture write failure"))
    }).await;
    assert_eq!(result.unwrap_err().to_string(), "fixture write failure");
    assert!(!profile.record().exists(), "partial JSON reached the final record");
    assert_eq!(profile.entries(), 0, "failed candidate was not removed");
    let id = DeviceRecord::load_or_create(&profile.0).await.unwrap();
    let saved = std::fs::read(profile.record()).unwrap();
    assert_eq!(DeviceRecord::load_or_create(&profile.0).await.unwrap(), id);
    assert_eq!(std::fs::read(profile.record()).unwrap(), saved);
    assert_eq!(profile.entries(), 1);
}

#[tokio::test]
async fn cancelled_publisher_cleans_up_without_replacing_a_later_identity() {
    let profile = Profile::new();
    let other = Profile::new();
    let mut abandoned = pending(profile.record(), LOSER.to_vec());
    let ready = timeout(Duration::from_secs(3), &mut abandoned.ready).await;
    abandoned.task.abort();
    let cancelled = timeout(Duration::from_secs(3), &mut abandoned.task).await;
    let unpublished = !profile.record().exists();
    let winner = create(profile.record(), WINNER.as_bytes().to_vec()).await;
    let other_id = DeviceRecord::load_or_create(&other.0).await;
    let released = abandoned.release.send(());
    let finished = timeout(Duration::from_secs(3), abandoned.finished).await;
    ready.unwrap().unwrap();
    assert!(cancelled.unwrap().unwrap_err().is_cancelled());
    finished.unwrap().unwrap();
    assert!(released.is_ok(), "blocking owner ended before the late publication");
    assert!(unpublished, "cancelled partial write exposed a record");
    assert_eq!(winner.unwrap(), WINNER);
    assert!(!other_id.unwrap().is_empty());
    assert_eq!(std::fs::read_to_string(profile.record()).unwrap(), WINNER);
    assert_eq!(DeviceRecord::load_or_create(&profile.0).await.unwrap(), "winner");
    assert_eq!(profile.entries(), 1, "abandoned candidate was not removed");
    assert_eq!(other.entries(), 1);
}
