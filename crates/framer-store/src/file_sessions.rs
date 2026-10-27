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

use async_trait::async_trait;
use framer_core::{ids::FramerUserId, port::{Session, SessionStore}};
use std::{fs::{File, OpenOptions}, io::{Read, Write}, path::PathBuf};

pub struct FileSessionStore {
    dir: PathBuf,
    _ownership: File,
    max_session_bytes: Option<usize>,
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
        Ok(Self { dir, _ownership: ownership, max_session_bytes: None })
    }

    /// Reject larger reads and saves without imposing a limit on other consumers.
    pub fn new_bounded(dir: impl Into<PathBuf>, max_bytes: usize) -> Result<Self, String> {
        max_bytes.checked_add(1).ok_or("session byte limit is too large")?;
        let mut store = Self::new(dir)?;
        store.max_session_bytes = Some(max_bytes);
        Ok(store)
    }

    fn path(&self, user: &FramerUserId) -> Result<PathBuf, String> {
        Ok(self.dir.join(session_filename(user, cfg!(windows))?))
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
        let result = (|| -> std::io::Result<()> {
            file.write_all(session.cookie_header.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&temporary, path)?;
            #[cfg(unix)]
            File::open(&self.dir)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() { let _ = std::fs::remove_file(temporary); }
        result.map_err(|e| e.to_string())
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
        match std::fs::remove_file(self.path(user)?) {
            Ok(()) => {
                #[cfg(unix)]
                File::open(&self.dir).and_then(|dir| dir.sync_all()).map_err(|e| e.to_string())?;
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        let mut options = OpenOptions::new();
        options.read(true);
        private_file(&mut options);
        match options.open(self.path(user)?) {
            Ok(mut file) => {
                require_regular_file(&file)?;
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
