use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

#[tokio::test]
async fn bounded_session_roundtrips_at_exact_byte_limit() {
    let root = Directory::new();
    let store = FileSessionStore::new_bounded(&root.0, 16 * 1024).unwrap();
    let session = Session { active_user: FramerUserId("fixture-user".into()),
        cookie_header: "é".repeat(8 * 1024) };
    store.save(&session).await.unwrap();
    assert_eq!(store.load(&session.active_user).await.unwrap().unwrap().cookie_header,
        session.cookie_header);
}

#[tokio::test]
async fn bounded_save_preserves_existing_session_on_overflow() {
    let root = Directory::new();
    let store = FileSessionStore::new_bounded(&root.0, 16 * 1024).unwrap();
    let mut session = Session { active_user: FramerUserId("fixture-user".into()),
        cookie_header: "session=fixture".into() };
    store.save(&session).await.unwrap();
    session.cookie_header = "x".repeat(16 * 1024 + 1);
    assert_eq!(store.save(&session).await.unwrap_err(), "session exceeds configured byte limit");
    assert_eq!(std::fs::read(store.path(&session.active_user).unwrap()).unwrap(), b"session=fixture");
    assert_eq!(store.load(&session.active_user).await.unwrap().unwrap().cookie_header, "session=fixture");
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 2);
}

#[tokio::test]
async fn bounded_load_rejects_existing_oversized_file() {
    let root = Directory::new();
    let user = FramerUserId("fixture-user".into());
    let large_header = "x".repeat(16 * 1024 + 1);
    // Seed the oversized file directly. This case tests bounded reading;
    // ownership handoff has separate process-lifecycle coverage.
    std::fs::write(root.0.join("fixture-user.session"), &large_header).unwrap();
    let bounded = FileSessionStore::new_bounded(&root.0, 16 * 1024).unwrap();
    assert_eq!(bounded.load(&user).await.unwrap_err(), "session exceeds configured byte limit");
    assert_eq!(std::fs::read(bounded.path(&user).unwrap()).unwrap(), large_header.as_bytes());
}

#[tokio::test]
async fn default_session_store_retains_large_header_support() {
    let root = Directory::new();
    let store = FileSessionStore::new(&root.0).unwrap();
    let session = Session { active_user: FramerUserId("fixture-user".into()),
        cookie_header: "x".repeat(16 * 1024 + 1) };
    store.save(&session).await.unwrap();
    assert_eq!(store.load(&session.active_user).await.unwrap().unwrap().cookie_header,
        session.cookie_header);
}

#[test]
fn bounded_reader_stops_at_limit_plus_one_when_more_data_remains() {
    struct ContinuousReader { bytes_read: usize }
    impl Read for ContinuousReader {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            bytes.fill(b'x');
            self.bytes_read += bytes.len();
            Ok(bytes.len())
        }
    }
    let mut reader = ContinuousReader { bytes_read: 0 };
    assert_eq!(read_bounded_session(&mut reader, 16 * 1024).unwrap_err(),
        "session exceeds configured byte limit");
    assert_eq!(reader.bytes_read, 16 * 1024 + 1);
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("framerize-session-test-{}-{}",
            std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)] {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

#[cfg(unix)]
fn make_fifo(path: &std::path::Path) {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // The CString keeps the NUL-terminated path alive for the entire call.
    let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
    assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
}

#[cfg(unix)]
#[test]
fn ownership_file_must_be_regular() {
    use std::os::unix::fs::FileTypeExt;
    let root = Directory::new();
    let path = root.0.join(".owner.lock");
    make_fifo(&path);
    assert_eq!(FileSessionStore::new(&root.0).err().as_deref(),
        Some("session store entry must be a regular file"));
    assert!(std::fs::metadata(path).unwrap().file_type().is_fifo());
}

#[cfg(unix)]
#[tokio::test]
async fn session_fifo_without_a_writer_is_refused() {
    // An async timeout cannot interrupt the old blocking open; the test process needs a deadline.
    let root = Directory::new();
    let store = FileSessionStore::new(&root.0).unwrap();
    let session = Session { active_user: FramerUserId("fixture-user".into()),
        cookie_header: "session=inert-fixture".into() };
    let path = store.path(&session.active_user).unwrap();
    make_fifo(&path);
    assert_eq!(store.load(&session.active_user).await.err().as_deref(),
        Some("session store entry must be a regular file"));
    std::fs::remove_file(path).unwrap();
    store.save(&session).await.unwrap();
    assert_eq!(store.load(&session.active_user).await.unwrap().unwrap().cookie_header,
        session.cookie_header);
}

#[cfg(unix)]
#[test]
fn existing_shared_session_directory_is_refused_without_modification() {
    use std::os::unix::fs::PermissionsExt;
    for mode in [0o740, 0o720, 0o710, 0o704, 0o702, 0o701] {
        let root = Directory::new();
        let saved = root.0.join("fixture.session");
        std::fs::write(&saved, b"inert-fixture").unwrap();
        std::fs::set_permissions(&root.0, std::fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(FileSessionStore::new(&root.0).err().as_deref(),
            Some("session directory must not allow group or other access"));
        assert!(!root.0.join(".owner.lock").exists());
        assert_eq!(std::fs::metadata(&root.0).unwrap().permissions().mode() & 0o777, mode);
        assert_eq!(std::fs::read(saved).unwrap(), b"inert-fixture");
    }
}

struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
}

fn child(root: &Directory, hold: bool) -> Child {
    use std::process::{Command, Stdio};
    Child(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "file_sessions::tests::child_process"])
        .env("FRAMERIZE_SESSION_FIXTURE", &root.0)
        .env("FRAMERIZE_SESSION_HOLD", if hold { "yes" } else { "no" })
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().unwrap())
}

#[test]
fn child_process() {
    let Some(root) = std::env::var_os("FRAMERIZE_SESSION_FIXTURE") else { return; };
    let root = PathBuf::from(root);
    let _store = match FileSessionStore::new(&root) {
        Ok(store) => store,
        Err(_) => std::process::exit(23),
    };
    if std::env::var("FRAMERIZE_SESSION_HOLD").unwrap() == "yes" {
        std::fs::write(root.join("ready"), b"owned").unwrap();
        let _ = std::io::stdin().read_exact(&mut [0]);
    }
}

#[test]
fn ownership_covers_alias_paths_until_last_store_reference_drops() {
    let root = Directory::new();
    let store = std::sync::Arc::new(FileSessionStore::new(&root.0).unwrap());
    let retained = store.clone();
    drop(store);
    assert!(FileSessionStore::new(root.0.join(".")).is_err());
    drop(retained);
    let reopened = FileSessionStore::new(&root.0).unwrap();
    assert!(root.0.join(".owner.lock").is_file());
    drop(reopened);
}

#[test]
fn another_process_is_refused_and_kernel_releases_a_dead_owner() {
    let root = Directory::new();
    let store = FileSessionStore::new(&root.0).unwrap();
    assert_eq!(child(&root, false).0.wait().unwrap().code(), Some(23));
    drop(store);
    let mut owner = child(&root, true);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !root.0.join("ready").exists() {
        assert!(owner.0.try_wait().unwrap().is_none(), "fixture exited before ownership");
        assert!(std::time::Instant::now() < deadline, "fixture ownership deadline");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(FileSessionStore::new(&root.0).is_err());
    owner.0.kill().unwrap();
    owner.0.wait().unwrap();
    assert!(FileSessionStore::new(&root.0).is_ok());
    assert!(root.0.join(".owner.lock").is_file());
}

#[tokio::test]
async fn replacement_is_private_and_does_not_truncate_the_previous_file() {
    let root = Directory::new();
    let store = FileSessionStore::new(root.0.join("sessions")).unwrap();
    let mut session = Session { active_user: FramerUserId("fixture-user".into()),
        cookie_header: "session=first-fixture".into() };
    store.save(&session).await.unwrap();
    let path = store.path(&session.active_user).unwrap();
    let mut previous = File::open(&path).unwrap();
    session.cookie_header = "session=second-fixture".into();
    store.save(&session).await.unwrap();
    assert_eq!(store.load(&session.active_user).await.unwrap().unwrap().cookie_header,
        session.cookie_header);
    let mut old = String::new();
    previous.read_to_string(&mut old).unwrap();
    assert_eq!(old, "session=first-fixture");
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&store.dir).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    store.delete(&session.active_user).await.unwrap();
    assert!(store.load(&session.active_user).await.unwrap().is_none());
    assert_eq!(std::fs::read_dir(&store.dir).unwrap().count(), 1);
}

#[tokio::test]
async fn invalid_user_names_cannot_address_another_directory() {
    let root = Directory::new();
    let store = FileSessionStore::new(&root.0).unwrap();
    for name in ["", "../outside", "/outside", "other\\user", "bad\0user"] {
        let user = FramerUserId(name.into());
        let session = Session { active_user: user.clone(), cookie_header: "fixture".into() };
        assert!(store.load(&user).await.is_err());
        assert!(store.save(&session).await.is_err());
        assert!(store.delete(&user).await.is_err());
    }
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn lock_and_session_reads_do_not_follow_file_symlinks() {
    use std::os::unix::fs::symlink;
    let root = Directory::new();
    let outside = root.0.join("outside");
    std::fs::write(&outside, "fixture").unwrap();
    symlink(&outside, root.0.join(".owner.lock")).unwrap();
    assert!(FileSessionStore::new(&root.0).is_err());
    std::fs::remove_file(root.0.join(".owner.lock")).unwrap();
    let store = FileSessionStore::new(&root.0).unwrap();
    let user = FramerUserId("fixture-user".into());
    symlink(&outside, store.path(&user).unwrap()).unwrap();
    assert!(store.load(&user).await.is_err());
    assert_eq!(std::fs::read_to_string(outside).unwrap(), "fixture");
}
