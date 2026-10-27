//! One process owns a session directory for its entire runtime lifetime.
#[cfg(test)]
#[path = "file_sessions_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "file_session_drive_prefix_tests.rs"]
mod drive_prefix_tests;
#[cfg(test)]
#[path = "file_session_temp_collision_tests.rs"]
mod temp_collision_tests;
#[cfg(all(test, unix))]
#[path = "file_session_delete_sync_tests.rs"]
mod delete_sync_tests;
#[cfg(test)]
#[path = "file_session_exact_identity_tests.rs"]
mod exact_identity_tests;

use async_trait::async_trait;
use framer_core::{ids::FramerUserId, port::{Session, SessionStore}};
use sha2::{Digest, Sha256};
use std::{fs::{File, OpenOptions}, io::{Read, Write}, path::PathBuf, sync::Mutex};

pub struct FileSessionStore {
    dir: PathBuf,
    _ownership: File,
    max_session_bytes: Option<usize>,
    mutation: Mutex<()>,
}

impl FileSessionStore {
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self, String> {
        let dir = dir.into();
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)] {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir).map_err(|e| e.to_string())?;
        let dir = dir.canonicalize().map_err(|e| e.to_string())?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            let permissions = std::fs::metadata(&dir).map_err(|e| e.to_string())?.permissions();
            if permissions.mode() & 0o077 != 0 {
                return Err("session directory must not allow group or other access".into());
            }
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        private_file(&mut options);
        let ownership = options.open(dir.join(".owner.lock")).map_err(|e| e.to_string())?;
        require_regular_file(&ownership)?;
        ownership.try_lock().map_err(|e| format!("session store ownership unavailable: {e}"))?;
        Ok(Self { dir, _ownership: ownership, max_session_bytes: None, mutation: Mutex::new(()) })
    }

    /// Reject larger reads and saves without imposing a limit on other consumers.
    pub fn new_bounded(dir: impl Into<PathBuf>, max_bytes: usize) -> Result<Self, String> {
        max_bytes.checked_add(1).ok_or("session byte limit is too large")?;
        let mut store = Self::new(dir)?;
        store.max_session_bytes = Some(max_bytes);
        Ok(store)
    }

    fn path(&self, user: &FramerUserId) -> Result<PathBuf, String> {
        session_filename(user, cfg!(windows))?;
        let mut digest = Sha256::new();
        digest.update(b"framerize-session-v2\0");
        digest.update(user.0.as_bytes());
        Ok(self.dir.join(format!("session-v2-{:x}.data", digest.finalize())))
    }
}

fn session_filename(user: &FramerUserId, windows: bool) -> Result<String, String> {
    if user.0.is_empty() || user.0.contains(['/', '\\', '\0']) {
        return Err("invalid session user".into());
    }
    // A Windows drive prefix makes PathBuf::join discard the owned directory.
    if windows && user.0.as_bytes().get(..2).is_some_and(|prefix| {
        prefix[0].is_ascii_alphabetic() && prefix[1] == b':'
    }) {
        return Err("invalid session user".into());
    }
    Ok(format!("{}.session", user.0))
}

fn record_read_error(error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        "session record identity conflict".into()
    } else { error.to_string() }
}

fn require_session_identity(reader: &mut impl Read, user: &FramerUserId) -> Result<(), String> {
    let mut header = [0; 16];
    reader.read_exact(&mut header).map_err(record_read_error)?;
    if &header[..8] != b"FRMSESS2" || header[8..] != (user.0.len() as u64).to_be_bytes() {
        return Err("session record identity conflict".into());
    }
    let mut buffer = [0; 1024];
    for expected in user.0.as_bytes().chunks(buffer.len()) {
        let actual = &mut buffer[..expected.len()];
        reader.read_exact(actual).map_err(record_read_error)?;
        if actual != expected { return Err("session record identity conflict".into()); }
    }
    Ok(())
}

fn private_file(options: &mut OpenOptions) {
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        // A FIFO must not block before its opened-file metadata can be checked.
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    #[cfg(not(unix))]
    let _ = options;
}

fn require_regular_file(file: &File) -> Result<(), String> {
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("session store entry must be a regular file".into());
    }
    Ok(())
}

fn check_existing_identity(path: &std::path::Path, user: &FramerUserId) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {},
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    }
    let mut options = OpenOptions::new();
    options.read(true);
    private_file(&mut options);
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    require_regular_file(&file)?;
    require_session_identity(&mut file, user)
}

fn write_session_record(writer: &mut impl Write, session: &Session) -> std::io::Result<()> {
    writer.write_all(b"FRMSESS2")?;
    writer.write_all(&(session.active_user.0.len() as u64).to_be_bytes())?;
    writer.write_all(session.active_user.0.as_bytes())?;
    writer.write_all(session.cookie_header.as_bytes())
}

fn read_bounded_session(reader: impl Read, max_bytes: usize) -> Result<String, String> {
    let read_limit = max_bytes.checked_add(1).ok_or("session byte limit is too large")?;
    let mut bytes = Vec::new();
    // Limit the read itself: metadata cannot bound a file that grows during reading.
    reader.take(read_limit as u64).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > max_bytes {
        return Err("session exceeds configured byte limit".into());
    }
    String::from_utf8(bytes).map_err(|_| "session file is not UTF-8".into())
}

impl FileSessionStore {
    fn save_with_sequence(&self, session: &Session, mut next: impl FnMut() -> u64) -> Result<(), String> {
        if self.max_session_bytes.is_some_and(|max| session.cookie_header.len() > max) {
            return Err("session exceeds configured byte limit".into());
        }
        let path = self.path(&session.active_user)?;
        let _mutation = self.mutation.lock().map_err(|_| "session store mutation unavailable")?;
        let (temporary, mut file) = loop {
            let temporary = self.dir.join(format!(".session-{}-{}.tmp",
                std::process::id(), next()));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            private_file(&mut options);
            match options.open(&temporary) {
                Ok(file) => break (temporary, file),
                // A previous process with this PID can leave a candidate behind.
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            }
        };
        let result = (|| -> Result<(), String> {
            write_session_record(&mut file, session).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            check_existing_identity(&path, &session.active_user)?;
            std::fs::rename(&temporary, path).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            File::open(&self.dir).and_then(|dir| dir.sync_all()).map_err(|e| e.to_string())?;
            Ok(())
        })();
        if result.is_err() { let _ = std::fs::remove_file(temporary); }
        result
    }

    fn delete_with_sync(&self, user: &FramerUserId,
        sync_directory: impl FnOnce(&std::path::Path) -> std::io::Result<()>) -> Result<(), String> {
        let path = self.path(user)?;
        let _mutation = self.mutation.lock().map_err(|_| "session store mutation unavailable")?;
        check_existing_identity(&path, user)?;
        match std::fs::remove_file(path) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.to_string()),
        }
        // A retry must finish synchronization even if the previous unlink succeeded.
        sync_directory(&self.dir).map_err(|error| error.to_string())
    }
}

#[async_trait]
impl SessionStore for FileSessionStore {
    async fn save(&self, session: &Session) -> Result<(), String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        self.save_with_sequence(session, || NEXT.fetch_add(1, Ordering::Relaxed))
    }

    async fn delete(&self, user: &FramerUserId) -> Result<(), String> {
        self.delete_with_sync(user, |dir| {
            #[cfg(unix)]
            { File::open(dir).and_then(|dir| dir.sync_all()) }
            #[cfg(not(unix))]
            {
                let _ = dir;
                Ok(())
            }
        })
    }

    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        let mut options = OpenOptions::new();
        options.read(true);
        private_file(&mut options);
        match options.open(self.path(user)?) {
            Ok(mut file) => {
                require_regular_file(&file)?;
                require_session_identity(&mut file, user)?;
                let cookie_header = if let Some(max_bytes) = self.max_session_bytes {
                    read_bounded_session(file, max_bytes)?
                } else {
                    let mut value = String::new();
                    file.read_to_string(&mut value).map_err(|e| e.to_string())?;
                    value
                };
                Ok(Some(Session { active_user: user.clone(), cookie_header }))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}
