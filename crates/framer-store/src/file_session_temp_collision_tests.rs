use super::*;
use std::path::Path;

struct Directory(PathBuf);

impl Drop for Directory {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn isolated<T>(run: impl FnOnce(&Path) -> T) -> T {
    let root = Directory(std::env::temp_dir().join(format!(
        "framerize-session-collision-{}", framer_core::ids::AccountId::new().0)));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)] {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&root.0).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&root.0)));
    let cleanup = std::fs::remove_dir_all(&root.0);
    assert!(cleanup.is_ok(), "fixture cleanup: {cleanup:?}");
    match result { Ok(value) => value, Err(error) => std::panic::resume_unwind(error) }
}

fn candidate(store: &FileSessionStore, sequence: u64) -> PathBuf {
    store.dir.join(format!(".session-{}-{sequence}.tmp", std::process::id()))
}

fn session() -> Session {
    Session { active_user: FramerUserId("fixture-user".into()),
        cookie_header: "session=new-inert-value".into() }
}

fn io<T>(result: std::io::Result<T>) -> Result<T, String> { result.map_err(|e| e.to_string()) }

#[test]
fn stale_candidates_do_not_block_session_replacement() {
    for stale in ["", "session=interrupted-inert-value"] {
        let observed = isolated(|root| -> Result<_, String> {
            let store = FileSessionStore::new(root)?;
            let session = session();
            let target = store.path(&session.active_user)?;
            io(std::fs::write(&target, super::exact_identity_tests::record(
                &session.active_user.0, b"session=previous-inert-value")))?;
            io(std::fs::write(candidate(&store, 0), stale))?;
            io(std::fs::write(candidate(&store, 1), b"another-stale-candidate"))?;
            let mut sequence = [0, 1, 2].into_iter();
            let mut calls = Vec::new();
            let result = store.save_with_sequence(&session, || {
                let next = sequence.next().expect("bounded candidate sequence exhausted");
                calls.push(next);
                next
            });
            Ok((result, calls, io(std::fs::read(target))?,
                io(std::fs::read(candidate(&store, 0)))?,
                io(std::fs::read(candidate(&store, 1)))?,
                candidate(&store, 2).exists(), io(std::fs::read_dir(root))?.count()))
        });
        let (result, calls, saved, first, second, temporary_exists, count) = observed.unwrap();
        assert_eq!(result, Ok(()));
        assert_eq!(calls, [0, 1, 2]);
        assert_eq!(saved, super::exact_identity_tests::record(
            "fixture-user", session().cookie_header.as_bytes()));
        assert_eq!(first, stale.as_bytes());
        assert_eq!(second, b"another-stale-candidate");
        assert!(!temporary_exists);
        assert_eq!(count, 4);
    }
}

#[cfg(unix)]
#[test]
fn occupied_nonfile_candidates_are_preserved() {
    use std::os::unix::{ffi::OsStrExt, fs::{symlink, FileTypeExt}};
    for kind in ["directory", "symlink", "fifo"] {
        let observed = isolated(|root| -> Result<_, String> {
            let store = FileSessionStore::new(root)?;
            let session = session();
            let target = store.path(&session.active_user)?;
            let protected = root.join("protected");
            let occupied = candidate(&store, 0);
            io(std::fs::write(&target, super::exact_identity_tests::record(
                &session.active_user.0, b"previous-inert-value")))?;
            io(std::fs::write(&protected, b"protected-inert-value"))?;
            match kind {
                "directory" => {
                    io(std::fs::create_dir(&occupied))?;
                    io(std::fs::write(occupied.join("nested"), b"nested-inert-value"))?;
                }
                "symlink" => io(symlink(&protected, &occupied))?,
                _ => {
                    let path = std::ffi::CString::new(occupied.as_os_str().as_bytes())
                        .map_err(|e| e.to_string())?;
                    // The CString keeps the path valid for this synchronous call.
                    if unsafe { libc::mkfifo(path.as_ptr(), 0o600) } != 0 {
                        return Err(std::io::Error::last_os_error().to_string());
                    }
                }
            }
            let mut sequence = [0, 1].into_iter();
            let result = store.save_with_sequence(&session,
                || sequence.next().expect("bounded candidate sequence exhausted"));
            let file_type = io(std::fs::symlink_metadata(&occupied))?.file_type();
            let retained = match kind {
                "directory" => file_type.is_dir()
                    && io(std::fs::read(occupied.join("nested")))? == b"nested-inert-value",
                "symlink" => file_type.is_symlink() && io(std::fs::read_link(&occupied))? == protected,
                _ => file_type.is_fifo(),
            };
            Ok((result, io(std::fs::read(target))?, retained,
                io(std::fs::read(protected))?, candidate(&store, 1).exists()))
        });
        let (result, saved, retained, protected, temporary_exists) = observed.unwrap();
        assert_eq!(result, Ok(()), "{kind}");
        assert_eq!(saved, super::exact_identity_tests::record(
            "fixture-user", session().cookie_header.as_bytes()));
        assert!(retained, "{kind}");
        assert_eq!(protected, b"protected-inert-value");
        assert!(!temporary_exists);
    }
}

#[test]
fn candidate_creation_errors_stop_without_replacing_the_session() {
    for kind in ["missing", "file"] {
        let observed = isolated(|root| -> Result<_, String> {
            let mut store = FileSessionStore::new(root.join("sessions"))?;
            let session = session();
            let saved = store.path(&session.active_user)?;
            io(std::fs::write(&saved, b"previous-inert-value"))?;
            let blocked = root.join("candidate-parent");
            if kind == "file" { io(std::fs::write(&blocked, b"parent-inert-value"))?; }
            // Inject an unavailable candidate directory without changing the real owner.
            store.dir = blocked.clone();
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            private_file(&mut options);
            let expected = options.open(candidate(&store, 0)).err()
                .ok_or("fixture unexpectedly created a candidate")?.to_string();
            let mut calls = 0;
            let mut sequence = [0].into_iter();
            let result = store.save_with_sequence(&session, || {
                calls += 1;
                sequence.next().expect("bounded candidate sequence exhausted")
            });
            let parent = if kind == "file" { Some(io(std::fs::read(blocked))?) } else { None };
            Ok((result, expected, calls, io(std::fs::read(saved))?, parent))
        });
        let (result, expected, calls, saved, parent) = observed.unwrap();
        assert_eq!(result, Err(expected));
        assert_eq!(calls, 1);
        assert_eq!(saved, b"previous-inert-value");
        if let Some(bytes) = parent { assert_eq!(bytes, b"parent-inert-value"); }
    }
}

#[test]
fn failed_publication_removes_only_the_owned_candidate() {
    let observed = isolated(|root| -> Result<_, String> {
        let store = FileSessionStore::new(root)?;
        let session = session();
        let target = store.path(&session.active_user)?;
        io(std::fs::create_dir(&target))?;
        io(std::fs::write(target.join("retained"), b"retained-inert-value"))?;
        io(std::fs::write(candidate(&store, 0), b"stale-inert-value"))?;
        let probe = root.join("error-probe");
        io(std::fs::write(&probe, b"probe"))?;
        let expected = std::fs::rename(&probe, &target).err()
            .ok_or("fixture unexpectedly replaced a directory")?.to_string();
        io(std::fs::remove_file(probe))?;
        let mut sequence = [0, 1].into_iter();
        let mut calls = Vec::new();
        let result = store.save_with_sequence(&session, || {
            let next = sequence.next().expect("bounded candidate sequence exhausted");
            calls.push(next);
            next
        });
        Ok((result, expected, calls, io(std::fs::read(candidate(&store, 0)))?,
            io(std::fs::read(target.join("retained")))?, candidate(&store, 1).exists(),
            io(std::fs::read_dir(root))?.count()))
    });
    let (result, expected, calls, stale, retained, temporary_exists, count) = observed.unwrap();
    assert_eq!(result, Err(expected));
    assert_eq!(calls, [0, 1]);
    assert_eq!(stale, b"stale-inert-value");
    assert_eq!(retained, b"retained-inert-value");
    assert!(!temporary_exists);
    assert_eq!(count, 3);
}
