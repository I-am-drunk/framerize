use super::*;
use std::path::Path;

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn isolated<T>(run: impl FnOnce(&Path) -> T) -> T {
    use std::os::unix::fs::DirBuilderExt;
    let root = Directory(std::env::temp_dir().join(format!(
        "framerize-session-delete-{}", framer_core::ids::AccountId::new().0)));
    std::fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&root.0)));
    let cleanup = std::fs::remove_dir_all(&root.0);
    assert!(cleanup.is_ok(), "fixture cleanup: {cleanup:?}");
    match result { Ok(value) => value, Err(error) => std::panic::resume_unwind(error) }
}

fn io<T>(result: std::io::Result<T>) -> Result<T, String> { result.map_err(|e| e.to_string()) }

fn sync_error() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::PermissionDenied, "inert directory sync failure")
}

fn absent(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.to_string()),
    }
}

#[test]
fn missing_session_retry_repeats_failed_directory_sync() {
    let observed = isolated(|root| -> Result<_, String> {
        let store = FileSessionStore::new(root)?;
        let user = FramerUserId("deleted-fixture-user".into());
        let target = store.path(&user)?;
        let peer = store.path(&FramerUserId("retained-fixture-user".into()))?;
        io(std::fs::write(&target, super::exact_identity_tests::record(
            &user.0, b"deleted-inert-value")))?;
        io(std::fs::write(&peer, b"retained-inert-value"))?;
        let mut calls = Vec::new();
        let first = store.delete_with_sync(&user, |dir| {
            calls.push(dir.to_path_buf());
            Err(sync_error())
        });
        let first_absent = absent(&target)?;
        let first_calls = calls.len();
        let retry = store.delete_with_sync(&user, |dir| {
            calls.push(dir.to_path_buf());
            File::open(dir).and_then(|file| file.sync_all())
        });
        Ok((first, first_absent, first_calls, retry, calls, store.dir.clone(),
            absent(&target)?, io(std::fs::read(peer))?, io(std::fs::read_dir(root))?.count()))
    });
    let (first, first_absent, first_calls, retry, calls, dir, gone, peer, count) = observed.unwrap();
    assert_eq!(first, Err(sync_error().to_string()));
    assert!(first_absent);
    assert_eq!(first_calls, 1);
    assert_eq!(retry, Ok(()));
    assert_eq!(calls, [dir.clone(), dir]);
    assert!(gone);
    assert_eq!(peer, b"retained-inert-value");
    assert_eq!(count, 2);
}

#[test]
fn initially_absent_session_preserves_directory_sync_errors() {
    let observed = isolated(|root| -> Result<_, String> {
        let store = FileSessionStore::new(root)?;
        let user = FramerUserId("absent-fixture-user".into());
        let peer = store.path(&FramerUserId("retained-fixture-user".into()))?;
        io(std::fs::write(&peer, b"retained-inert-value"))?;
        let mut calls = Vec::new();
        let result = store.delete_with_sync(&user, |dir| {
            calls.push(dir.to_path_buf());
            Err(sync_error())
        });
        Ok((result, calls, store.dir.clone(), absent(&store.path(&user)?)?,
            io(std::fs::read(peer))?, io(std::fs::read_dir(root))?.count()))
    });
    let (result, calls, dir, gone, peer, count) = observed.unwrap();
    assert_eq!(result, Err(sync_error().to_string()));
    assert_eq!(calls, [dir]);
    assert!(gone);
    assert_eq!(peer, b"retained-inert-value");
    assert_eq!(count, 2);
}

#[test]
fn validation_and_removal_errors_precede_directory_sync() {
    for kind in ["invalid-user", "directory"] {
        let observed = isolated(|root| -> Result<_, String> {
            let store = FileSessionStore::new(root)?;
            let peer = root.join("retained.session");
            io(std::fs::write(&peer, b"retained-inert-value"))?;
            let name = if kind == "invalid-user" { "../outside" } else { "directory" };
            let user = FramerUserId(name.into());
            let target = if kind == "directory" { store.path(&user)? }
                else { root.join("directory.session") };
            let expected = if kind == "directory" {
                io(std::fs::create_dir(&target))?;
                io(std::fs::write(target.join("retained"), b"directory-inert-value"))?;
                std::fs::remove_file(&target).err().ok_or("fixture removed a directory")?.to_string()
            } else { "invalid session user".into() };
            let mut calls = 0;
            let result = store.delete_with_sync(&user, |_| {
                calls += 1;
                Err(sync_error())
            });
            let nested = if kind == "directory" {
                Some(io(std::fs::read(target.join("retained")))?)
            } else { None };
            Ok((result, expected, calls, io(std::fs::read(peer))?, nested))
        });
        let (result, expected, calls, peer, nested) = observed.unwrap();
        assert_eq!(result, Err(expected));
        assert_eq!(calls, 0);
        assert_eq!(peer, b"retained-inert-value");
        if let Some(value) = nested { assert_eq!(value, b"directory-inert-value"); }
    }
}
