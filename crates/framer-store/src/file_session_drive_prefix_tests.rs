use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

const DRIVE_USERS: &[(&str, &str)] = &[
    ("C:", "C:.session"),
    ("C:peer", "C:peer.session"),
    ("c:peer", "c:peer.session"),
    ("Z:用户", "Z:用户.session"),
    ("m:.hidden", "m:.hidden.session"),
    ("Q:peer:stream", "Q:peer:stream.session"),
];

#[test]
fn selected_windows_platform_rejects_drive_prefixes() {
    for (user, _) in DRIVE_USERS {
        assert_eq!(session_filename(&FramerUserId((*user).into()), true),
            Err("invalid session user".into()), "{user}");
    }
}

#[test]
fn selected_posix_platform_preserves_colon_names() {
    for (user, expected) in DRIVE_USERS {
        assert_eq!(session_filename(&FramerUserId((*user).into()), false),
            Ok((*expected).into()), "{user}");
    }
}

#[test]
fn non_drive_names_and_original_errors_are_preserved() {
    let ordinary = [
        ("d55387f7-0180-4f92-a3f8-bceff0522a41", "d55387f7-0180-4f92-a3f8-bceff0522a41.session"),
        ("fixture-user", "fixture-user.session"),
        ("用户", "用户.session"),
        ("1:peer", "1:peer.session"),
        ("peer:tag", "peer:tag.session"),
    ];
    for windows in [false, true] {
        for (user, expected) in ordinary {
            assert_eq!(session_filename(&FramerUserId(user.into()), windows),
                Ok(expected.into()), "platform={windows}, user={user}");
        }
        for user in ["", "../outside", "/outside", "other\\user", "bad\0user"] {
            assert_eq!(session_filename(&FramerUserId(user.into()), windows),
                Err("invalid session user".into()), "platform={windows}");
        }
    }
}

#[test]
fn host_path_uses_the_build_platform() {
    let root = Directory::new();
    let store = FileSessionStore::new(&root.0).unwrap();
    let addresses = [
        "session-v2-528aab892e3efcd8feec8f0a30b35adcc3c53eef5892a0c427f6affca078eb1d.data",
        "session-v2-d0b24d0b949fda4d0f7ee05ca39efaa9b90ab00167f64ed185863c33a23e6980.data",
        "session-v2-b9f1df1f8d76d8a9425ebfc0feb17d6e742cade5e700544b419a5cf347fa01f3.data",
        "session-v2-88c525b317bac6d4046722ba3eb32c37315085b239515b1eda29e8121d3db375.data",
        "session-v2-07e4632d887763cd6bf29692c78e8f00ed19d3b768ea3d91f86982cca0b6e70d.data",
        "session-v2-ef3ba3f42f71cbbad9465981521f5dde36b8fde1521d172942b4c3f1593d1628.data",
    ];
    for (index, (user, _)) in DRIVE_USERS.iter().enumerate() {
        let expected = addresses[index];
        let result = store.path(&FramerUserId((*user).into()));
        if cfg!(windows) {
            assert_eq!(result, Err("invalid session user".into()));
        } else {
            assert_eq!(result, Ok(store.dir.join(expected)));
        }
    }
    // No drive-relative path is opened, even if this assertion regresses.
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 1);
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("framerize-session-drive-{}-{}",
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
