use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("framerize-session-test-{}-{}",
            std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
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
