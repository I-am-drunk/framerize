//! One process owns a session directory for its entire runtime lifetime.
#[cfg(test)]
#[path = "file_sessions_tests.rs"]
mod tests;

use async_trait::async_trait;
use framer_core::{ids::FramerUserId, port::{Session, SessionStore}};
use std::{fs::{File, OpenOptions}, io::{Read, Write}, path::PathBuf};

pub struct FileSessionStore {
    dir: PathBuf,
    _ownership: File,
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
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        private_file(&mut options);
        let ownership = options.open(dir.join(".owner.lock")).map_err(|e| e.to_string())?;
        ownership.try_lock().map_err(|e| format!("session store ownership unavailable: {e}"))?;
        Ok(Self { dir, _ownership: ownership })
    }

    fn path(&self, user: &FramerUserId) -> Result<PathBuf, String> {
        if user.0.is_empty() || user.0.contains(['/', '\\', '\0']) {
            return Err("invalid session user".into());
        }
        Ok(self.dir.join(format!("{}.session", user.0)))
    }
}

fn private_file(options: &mut OpenOptions) {
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(not(unix))]
    let _ = options;
}

#[async_trait]
impl SessionStore for FileSessionStore {
    async fn save(&self, session: &Session) -> Result<(), String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = self.path(&session.active_user)?;
        let temporary = self.dir.join(format!(".session-{}-{}.tmp",
            std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        private_file(&mut options);
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
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
                let mut cookie_header = String::new();
                file.read_to_string(&mut cookie_header).map_err(|e| e.to_string())?;
                Ok(Some(Session { active_user: user.clone(), cookie_header }))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}
