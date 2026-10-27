use super::*;
use std::{future::Future, path::Path, task::{Context, Poll, Waker}};

pub(super) fn record(user: &str, cookie: &[u8]) -> Vec<u8> {
    let mut bytes = b"FRMSESS2".to_vec();
    bytes.extend_from_slice(&(user.len() as u64).to_be_bytes());
    bytes.extend_from_slice(user.as_bytes());
    bytes.extend_from_slice(cookie);
    bytes
}

fn complete<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("file session fixture unexpectedly suspended"),
    }
}

fn session(user: &str, cookie: &str) -> Session {
    Session { active_user: FramerUserId(user.into()), cookie_header: cookie.into() }
}

fn load(store: &FileSessionStore, user: &str) -> Result<Option<(String, String)>, String> {
    complete(store.load(&FramerUserId(user.into())))
        .map(|value| value.map(|session| (session.active_user.0, session.cookie_header)))
}

fn io<T>(result: std::io::Result<T>) -> Result<T, String> { result.map_err(|e| e.to_string()) }

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn isolated<T>(run: impl FnOnce(&Path) -> T) -> T {
    let root = Directory(std::env::temp_dir().join(format!(
        "framerize-session-exact-{}", framer_core::ids::AccountId::new().0)));
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

const UPPER: &str = "session-v2-4fb4ad61e72a2a1d37ae3685ec6cbea60f83eadec56b3776185cbb4af2a36cb4.data";
const LOWER: &str = "session-v2-b606109a223091877795db3eb4f6d20ac0ad9da36026625c8eec736b682838cf.data";
const CONFLICT: &str = "session record identity conflict";

#[test]
fn exact_string_keys_have_casefold_safe_record_paths() {
    let vectors = [
        ("A".into(), "4fb4ad61e72a2a1d37ae3685ec6cbea60f83eadec56b3776185cbb4af2a36cb4"),
        ("a".into(), "b606109a223091877795db3eb4f6d20ac0ad9da36026625c8eec736b682838cf"),
        ("é".into(), "0d46d4cf021ad6ad439b4ae88a2671e00a1b85dbf916e048f0eac03e064ea8df"),
        ("e\u{301}".into(), "a5c9128f10f8411fceaa53d4e9000b4eca0837318a11fd2189cecf7e0200f5cf"),
        ("user-full-a".into(), "d00b5a82a08b5faad7823982435cc1391f8b7cccee5b022c93e4e92c629b4d79"),
        ("user-full-b".into(), "9a3f87c7da086ca14ce24cf5f513b09fbdd7b78a7d7727088f5b625fda8c2735"),
        ("CON".into(), "43c83abfc5e5bcf9e9674839baf36effb6680a8ead67b0d5360d2b25045cd310"),
        ("user.".into(), "f53d6c7bb0d9b92406a5b35cf03b6f6b341962f22fab3be1772b3f1de55e80be"),
        ("user ".into(), "daa140973639f19bb5cfe01c2e6c1bb9cc1bb6c735c8393dfe3a856f3460cd24"),
        ("x".repeat(300), "dfd17f95442784af37189fa03cb7b0ee2e3bff8621eada0c0bac0992e3bc4f20"),
        (format!("{}X", "a".repeat(2048)), "ac5cba57d3012c89cd4122217056baf9336cb6973414726e925cf0d218088244"),
        (format!("{}x", "a".repeat(2048)), "f39854f5e5c6ea5f3809e892066fe3f89c30af073694d53688c331b20e7ac589"),
    ];
    let mut paths = std::collections::HashSet::new();
    for (user, digest) in vectors {
        let observed = isolated(|root| -> Result<_, String> {
            let store = FileSessionStore::new(root)?;
            let value = session(&user, "session=inert-é");
            let path = store.path(&value.active_user)?;
            let published = complete(store.save(&value));
            let loaded = load(&store, &user);
            let bytes = io(std::fs::read(&path));
            Ok((path.file_name().unwrap().to_string_lossy().into_owned(),
                published, loaded, bytes, io(std::fs::read_dir(root))?.count()))
        });
        let (name, published, loaded, bytes, count) = observed.unwrap();
        assert_eq!(name, format!("session-v2-{digest}.data"));
        assert!(paths.insert(name.to_ascii_lowercase()));
        assert_eq!(published, Ok(()));
        assert_eq!(loaded, Ok(Some((user.clone(), "session=inert-é".into()))));
        assert_eq!(bytes, Ok(record(&user, "session=inert-é".as_bytes())));
        assert_eq!(count, 2);
    }
}

#[test]
fn legacy_files_require_reauthentication_without_adoption() {
    for bytes in [b"session=legacy-inert".to_vec(), vec![0xff, 0], Vec::new()] {
        let observed = isolated(|root| -> Result<_, String> {
            let store = FileSessionStore::new(root)?;
            let legacy = root.join("A.session");
            io(std::fs::write(&legacy, &bytes))?;
            Ok((load(&store, "A"), load(&store, "a"), io(std::fs::read(legacy))?,
                root.join(UPPER).exists(), root.join(LOWER).exists(),
                io(std::fs::read_dir(root))?.count()))
        });
        let (upper, lower, retained, upper_exists, lower_exists, count) = observed.unwrap();
        assert_eq!(upper, Ok(None));
        assert_eq!(lower, Ok(None));
        assert_eq!(retained, bytes);
        assert!(!upper_exists && !lower_exists);
        assert_eq!(count, 2);
    }
    let observed = isolated(|root| -> Result<_, String> {
        let store = FileSessionStore::new(root)?;
        let legacy = root.join("A.session");
        io(std::fs::create_dir(&legacy))?;
        io(std::fs::write(legacy.join("untouched"), b"legacy-directory"))?;
        Ok((load(&store, "A"), io(std::fs::read(legacy.join("untouched")))?,
            io(std::fs::read_dir(root))?.count()))
    });
    let (loaded, retained, count) = observed.unwrap();
    assert_eq!(loaded, Ok(None));
    assert_eq!(retained, b"legacy-directory");
    assert_eq!(count, 2);
}

#[test]
fn fresh_publication_and_delete_preserve_legacy_files() {
    let observed = isolated(|root| -> Result<_, String> {
        let store = FileSessionStore::new(root)?;
        io(std::fs::write(root.join("A.session"), b"session=legacy-inert"))?;
        io(std::fs::write(root.join("peer.session"), b"session=legacy-peer"))?;
        let value = session("A", "session=fresh-inert");
        let saved = complete(store.save(&value));
        let old_after_save = io(std::fs::read(root.join("A.session")));
        let new_after_save = io(std::fs::read(root.join(UPPER)));
        let loaded = load(&store, "A");
        let save_count = io(std::fs::read_dir(root))?.count();
        let deleted = complete(store.delete(&value.active_user));
        let old_after_delete = io(std::fs::read(root.join("A.session")));
        let peer = io(std::fs::read(root.join("peer.session")))?;
        let new_exists = root.join(UPPER).exists();
        drop(store);
        let reopened = FileSessionStore::new(root)?;
        Ok((saved, old_after_save, new_after_save, loaded, save_count, deleted,
            old_after_delete, peer, new_exists, load(&reopened, "A"),
            io(std::fs::read_dir(root))?.count()))
    });
    let (saved, old_saved, new_saved, loaded, save_count, deleted, old_deleted,
        peer, new_exists, reopened, delete_count) = observed.unwrap();
    assert_eq!(old_saved, Ok(b"session=legacy-inert".to_vec()));
    assert_eq!(saved, Ok(()));
    assert_eq!(new_saved, Ok(record("A", b"session=fresh-inert")));
    assert_eq!(loaded, Ok(Some(("A".into(), "session=fresh-inert".into()))));
    assert_eq!(save_count, 4);
    assert_eq!(deleted, Ok(()));
    assert_eq!(old_deleted, Ok(b"session=legacy-inert".to_vec()));
    assert_eq!(peer, b"session=legacy-peer");
    assert!(!new_exists);
    assert_eq!(reopened, Ok(None));
    assert_eq!(delete_count, 3);
}

#[test]
fn foreign_record_cannot_be_loaded_replaced_or_deleted() {
    for (user, foreign) in [("A".to_string(), "a".to_string()),
        (format!("{}X", "a".repeat(2048)), format!("{}x", "a".repeat(2048)))] {
        let bytes = record(&foreign, b"session=foreign-inert");
        let observed = isolated(|root| -> Result<_, String> {
            let store = FileSessionStore::new(root)?;
            let value = session(&user, "session=replacement-inert");
            let target = store.path(&value.active_user)?;
            io(std::fs::write(&target, &bytes))?;
            let peer_bytes = record("peer", b"session=peer-inert");
            io(std::fs::write(root.join("peer.data"), &peer_bytes))?;
            let loaded = load(&store, &user);
            let saved = complete(store.save(&value));
            let after_save = io(std::fs::read(&target));
            let deleted = complete(store.delete(&value.active_user));
            Ok((loaded, saved, after_save, deleted, io(std::fs::read(target)),
                io(std::fs::read(root.join("peer.data")))?, peer_bytes,
                io(std::fs::read_dir(root))?.count()))
        });
        let (loaded, saved, after_save, deleted, after_delete, peer, peer_bytes, count) = observed.unwrap();
        assert_eq!(loaded, Err(CONFLICT.into()));
        assert_eq!(saved, Err(CONFLICT.into()));
        assert_eq!(after_save, Ok(bytes.clone()));
        assert_eq!(deleted, Err(CONFLICT.into()));
        assert_eq!(after_delete, Ok(bytes));
        assert_eq!(peer, peer_bytes);
        assert_eq!(count, 3);
    }
}

#[test]
fn malformed_record_headers_cannot_be_relabelled() {
    let mut wrong_magic = record("A", b"cookie");
    wrong_magic[0] = b'X';
    let mut excessive = b"FRMSESS2".to_vec();
    excessive.extend_from_slice(&u64::MAX.to_be_bytes());
    let mut too_short = record("A", b"");
    too_short.truncate(16);
    let mut zero_length = record("", b"cookie");
    zero_length.push(0);
    let cases = [Vec::new(), b"FRMSESS2".to_vec(), wrong_magic, too_short,
        excessive, zero_length, record("AA", b"cookie")];
    for bytes in cases {
        let observed = isolated(|root| -> Result<_, String> {
            let store = FileSessionStore::new(root)?;
            let value = session("A", "session=new-inert");
            let target = store.path(&value.active_user)?;
            io(std::fs::write(&target, &bytes))?;
            let loaded = load(&store, "A");
            let saved = complete(store.save(&value));
            let after_save = io(std::fs::read(&target));
            let deleted = complete(store.delete(&value.active_user));
            Ok((loaded, saved, after_save, deleted, io(std::fs::read(target)),
                io(std::fs::read_dir(root))?.count()))
        });
        let (loaded, saved, after_save, deleted, after_delete, count) = observed.unwrap();
        assert_eq!(loaded, Err(CONFLICT.into()));
        assert_eq!(saved, Err(CONFLICT.into()));
        assert_eq!(after_save, Ok(bytes.clone()));
        assert_eq!(deleted, Err(CONFLICT.into()));
        assert_eq!(after_delete, Ok(bytes));
        assert_eq!(count, 2);
    }
}

#[test]
fn record_envelope_preserves_cookie_byte_limits() {
    for (user, cookie) in [("A".into(), "éé"), ("é".into(), ""),
        (format!("{}X", "a".repeat(2048)), "inert=\0é")] {
        let observed = isolated(|root| -> Result<_, String> {
            let store = FileSessionStore::new_bounded(root, cookie.len())?;
            let value = session(&user, cookie);
            let path = store.path(&value.active_user)?;
            let saved = complete(store.save(&value));
            let bytes = io(std::fs::read(&path));
            let loaded = load(&store, &user);
            let oversized = format!("{cookie}x");
            let overflow = complete(store.save(&session(&user, &oversized)));
            let invalid_overflow = complete(store.save(&session("", &oversized)));
            let after_overflow = io(std::fs::read(&path));
            io(std::fs::write(&path, record(&user, oversized.as_bytes())))?;
            let read_overflow = load(&store, &user);
            Ok((saved, bytes, loaded, overflow, invalid_overflow, after_overflow,
                read_overflow, io(std::fs::read(path))?, io(std::fs::read_dir(root))?.count()))
        });
        let (saved, bytes, loaded, overflow, invalid_overflow, after_overflow,
            read_overflow, oversized_bytes, count) = observed.unwrap();
        assert_eq!(bytes, Ok(record(&user, cookie.as_bytes())));
        assert_eq!(saved, Ok(()));
        assert_eq!(loaded, Ok(Some((user.clone(), cookie.into()))));
        assert_eq!(overflow, Err("session exceeds configured byte limit".into()));
        assert_eq!(invalid_overflow, Err("session exceeds configured byte limit".into()));
        assert_eq!(after_overflow, Ok(record(&user, cookie.as_bytes())));
        assert_eq!(read_overflow, Err("session exceeds configured byte limit".into()));
        assert_eq!(oversized_bytes, record(&user, format!("{cookie}x").as_bytes()));
        assert_eq!(count, 2);
    }
    for bound in [None, Some(8)] {
        let observed = isolated(|root| -> Result<_, String> {
            let store = match bound {
                Some(limit) => FileSessionStore::new_bounded(root, limit)?,
                None => FileSessionStore::new(root)?,
            };
            let path = store.path(&FramerUserId("A".into()))?;
            io(std::fs::write(&path, record("A", &[0xff])))?;
            let loaded = load(&store, "A");
            let bytes = io(std::fs::read(&path))?;
            io(std::fs::write(&path, record("a", &[0xff; 32])))?;
            Ok((loaded, bytes, load(&store, "A")))
        });
        let (loaded, bytes, conflict) = observed.unwrap();
        let raw_error = std::io::Cursor::new([0xff]).read_to_string(&mut String::new())
            .unwrap_err().to_string();
        let expected = if bound.is_some() { "session file is not UTF-8".into() } else { raw_error };
        assert_eq!(loaded, Err(expected));
        assert_eq!(bytes, record("A", &[0xff]));
        assert_eq!(conflict, Err(CONFLICT.into()));
    }
}

fn mutation_order(first_deletes: bool, second_deletes: bool) {
    use std::sync::{mpsc, Arc};
    use std::time::Duration;
    let observed = isolated(|root| -> Result<_, String> {
        let store = Arc::new(FileSessionStore::new(root)?);
        complete(store.save(&session("A", "session=initial-inert")))?;
        let (held_tx, held_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (progress_tx, progress_rx) = mpsc::sync_channel(1);
        let bound = Duration::from_secs(3);
        let first_store = store.clone();
        let second_store = store.clone();
        let observations = std::thread::scope(|scope| {
            let first = scope.spawn(move || {
                let mut released = false;
                let mut hold = || {
                    let _ = held_tx.try_send(());
                    released = release_rx.recv_timeout(bound).is_ok();
                };
                let value = session("A", "session=first-inert");
                let result = if first_deletes {
                    first_store.delete_with_sync(&value.active_user, |_| { hold(); Ok(()) })
                } else {
                    first_store.save_with_sequence(&value, || { hold(); 41 })
                };
                (result, released)
            });
            let held = held_rx.recv_timeout(bound);
            let second = scope.spawn(move || {
                let _ = started_tx.try_send(());
                let value = session("A", "session=second-inert");
                let progress = || { let _ = progress_tx.try_send(()); };
                if second_deletes {
                    second_store.delete_with_sync(&value.active_user, |_| { progress(); Ok(()) })
                } else {
                    second_store.save_with_sequence(&value, || { progress(); 42 })
                }
            });
            let started = started_rx.recv_timeout(bound);
            let early = progress_rx.recv_timeout(Duration::from_millis(250));
            let released = release_tx.try_send(()).is_ok();
            drop(release_tx);
            let first = first.join().map_err(|_| "first fixture thread panicked");
            let second = second.join().map_err(|_| "second fixture thread panicked");
            drop((held_rx, started_rx, progress_rx));
            (held, started, early, released, first, second)
        });
        Ok((observations, load(&store, "A"), io(std::fs::read_dir(root))?.count()))
    });
    let ((held, started, early, sent, first, second), loaded, count) = observed.unwrap();
    assert_eq!(early, Err(mpsc::RecvTimeoutError::Timeout));
    assert_eq!(held, Ok(()));
    assert_eq!(started, Ok(()));
    assert!(sent);
    let (first, released) = first.unwrap();
    assert!(released);
    assert_eq!(first, Ok(()));
    assert_eq!(second, Ok(Ok(())));
    let expected = if second_deletes { None }
        else { Some(("A".into(), "session=second-inert".into())) };
    assert_eq!(loaded, Ok(expected));
    assert_eq!(count, if second_deletes { 1 } else { 2 });
}

#[test]
fn same_store_mutations_serialize_check_and_publication() {
    for (first_deletes, second_deletes) in [(false, false), (false, true), (true, false)] {
        mutation_order(first_deletes, second_deletes);
    }
    let observed = isolated(|root| -> Result<_, String> {
        let store = FileSessionStore::new_bounded(root, 16)?;
        complete(store.save(&session("A", "inert=old")))?;
        let path = store.path(&FramerUserId("A".into()))?;
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.save_with_sequence(&session("A", "inert=new"), || panic!("inert sequence failure"))
        })).is_err();
        let saved = complete(store.save(&session("A", "inert=new")));
        let deleted = complete(store.delete(&FramerUserId("A".into())));
        let invalid_save = complete(store.save(&session("", "inert")));
        let invalid_delete = complete(store.delete(&FramerUserId("".into())));
        let overflow = complete(store.save(&session("", &"x".repeat(17))));
        Ok((poisoned, saved, deleted, invalid_save, invalid_delete, overflow,
            load(&store, "A"), io(std::fs::read(path))?, io(std::fs::read_dir(root))?.count()))
    });
    let (poisoned, saved, deleted, invalid_save, invalid_delete, overflow, loaded, bytes, count) = observed.unwrap();
    assert!(poisoned);
    assert_eq!(saved, Err("session store mutation unavailable".into()));
    assert_eq!(deleted, Err("session store mutation unavailable".into()));
    assert_eq!(invalid_save, Err("invalid session user".into()));
    assert_eq!(invalid_delete, Err("invalid session user".into()));
    assert_eq!(overflow, Err("session exceeds configured byte limit".into()));
    assert_eq!(loaded, Ok(Some(("A".into(), "inert=old".into()))));
    assert_eq!(bytes, record("A", b"inert=old"));
    assert_eq!(count, 2);
}

#[test]
fn same_identity_refresh_preserves_old_reader_and_peers() {
    let observed = isolated(|root| -> Result<_, String> {
        let store = FileSessionStore::new(root)?;
        complete(store.save(&session("A", "session=old-inert")))?;
        complete(store.save(&session("a", "session=peer-inert")))?;
        let path = store.path(&FramerUserId("A".into()))?;
        let peer = store.path(&FramerUserId("a".into()))?;
        let mut previous = io(File::open(&path))?;
        let refreshed = complete(store.save(&session("A", "session=new-inert")));
        let mut old = Vec::new();
        io(previous.read_to_end(&mut old))?;
        drop(previous);
        let current = io(std::fs::read(path))?;
        let peer_bytes = io(std::fs::read(peer))?;
        let loaded = load(&store, "A");
        let peer_loaded = load(&store, "a");
        drop(store);
        let reopened = FileSessionStore::new(root)?;
        Ok((refreshed, old, current, peer_bytes, loaded, peer_loaded,
            load(&reopened, "A"), load(&reopened, "a"), io(std::fs::read_dir(root))?.count()))
    });
    let (refreshed, old, current, peer, loaded, peer_loaded, reopened, peer_reopened, count) = observed.unwrap();
    assert_eq!(old, record("A", b"session=old-inert"));
    assert_eq!(refreshed, Ok(()));
    assert_eq!(current, record("A", b"session=new-inert"));
    assert_eq!(peer, record("a", b"session=peer-inert"));
    assert_eq!(loaded, Ok(Some(("A".into(), "session=new-inert".into()))));
    assert_eq!(peer_loaded, Ok(Some(("a".into(), "session=peer-inert".into()))));
    assert_eq!(reopened, loaded);
    assert_eq!(peer_reopened, peer_loaded);
    assert_eq!(count, 3);
}
