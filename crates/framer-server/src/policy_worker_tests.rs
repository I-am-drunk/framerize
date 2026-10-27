use super::*;

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("framer-worker-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn executable(&self, bytes: &[u8]) -> Executable {
        let path = self.0.join("worker");
        std::fs::write(&path, bytes).unwrap();
        Executable { path, sha256: Sha256::digest(bytes).into() }
    }
}
impl Drop for Scratch {
    fn drop(&mut self) { std::fs::remove_dir_all(&self.0).unwrap(); }
}

#[cfg(target_os = "linux")]
#[test]
fn executable_verification_rejects_mismatch_non_elf_and_oversize() {
    let scratch = Scratch::new();
    let mut descriptor = scratch.executable(b"\x7fELF-synthetic");
    descriptor.sha256 = [0; 32];
    assert!(matches!(verify(&descriptor), Err(ProcessError::Executable)));
    let descriptor = scratch.executable(b"#!/bin/sh\n");
    assert!(matches!(verify(&descriptor), Err(ProcessError::Executable)));
    let descriptor = scratch.executable(b"\x7fELF-synthetic");
    std::fs::OpenOptions::new().write(true).open(&descriptor.path).unwrap()
        .set_len(MAX_EXECUTABLE_BYTES + 1).unwrap();
    assert!(matches!(verify(&descriptor), Err(ProcessError::Executable)));
    let relative = Executable { path: "worker".into(), sha256: [0; 32] };
    assert!(matches!(verify(&relative), Err(ProcessError::Executable)));
}

#[cfg(target_os = "linux")]
#[test]
fn opened_executable_keeps_the_hashed_inode_after_path_replacement() {
    use std::io::{Seek, SeekFrom};
    use std::os::fd::AsRawFd;
    let scratch = Scratch::new();
    let original = b"\x7fELF-original-synthetic-inode";
    let executable = scratch.executable(original);
    let mut file = verify(&executable).unwrap();
    assert!(file.as_raw_fd() > 2);
    std::fs::rename(&executable.path, scratch.0.join("old")).unwrap();
    std::fs::write(&executable.path, b"\x7fELF-replacement").unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut retained = Vec::new();
    file.read_to_end(&mut retained).unwrap();
    assert_eq!(retained, original);
    assert!(verify(&executable).is_err());
    let command = command(&file);
    assert_eq!(command.get_program(), std::ffi::OsStr::new(
        &format!("/proc/self/fd/{}", file.as_raw_fd())));
    assert_eq!(command.get_current_dir(), Some(std::path::Path::new("/")));
    assert_eq!(command.get_args().count(), 0);
}

#[tokio::test]
async fn stderr_is_counted_and_discarded_without_retaining_payloads() {
    let counts = Arc::new(StderrCount::default());
    let bytes = vec![0xa5; STDERR_RESERVE * 7 + 1];
    drain_stderr(bytes.as_slice(), counts.clone()).await;
    assert_eq!(counts.read.load(Ordering::Relaxed), bytes.len() as u64);
}

#[tokio::test]
async fn bounded_dispatch_refusal_stops_before_the_next_frame() {
    let first = frame::encode(&Message::Ready { generation: 7 }).unwrap();
    let second = frame::encode(&Message::Ready { generation: 8 }).unwrap();
    let bytes = [first, second.clone()].concat();
    let mut input = bytes.as_slice();
    let seen = Arc::new(AtomicU64::new(0));
    let count = seen.clone();
    let dispatch: Dispatch = Arc::new(move |received| {
        assert_eq!(received.message, Message::Ready { generation: 7 });
        count.fetch_add(1, Ordering::Relaxed);
        Err(WireError::Protocol)
    });
    assert!(matches!(read_frames(&mut input, dispatch).await, Err(WireError::Protocol)));
    assert_eq!(seen.load(Ordering::Relaxed), 1);
    assert_eq!(input, second);
}
