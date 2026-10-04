//! Persistence, split by sensitivity (ADR-001):
//!   - `SqliteStateStore` holds non-secret account state. It has NO secret columns.
//!   - `KeyringSessionStore` holds the `session` cookie in the OS keychain, keyed
//!     by Framer user id.

use async_trait::async_trait;
use framer_core::account::ManagedAccount;
use framer_core::ids::{AccountId, FramerUserId};
use framer_core::port::{Session, SessionStore, StateStore};
use rusqlite::Connection;
use std::sync::Mutex;

const KEYCHAIN_SERVICE: &str = "framerize.session";

pub struct SqliteStateStore {
    conn: Mutex<Connection>,
}

impl SqliteStateStore {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS accounts (
                id            TEXT PRIMARY KEY,
                json          TEXT NOT NULL
             );",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { conn: Mutex::new(conn) })
    }
}

#[async_trait]
impl StateStore for SqliteStateStore {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        let conn = self.conn.lock().unwrap();
        let row: Option<String> = conn
            .query_row(
                "SELECT json FROM accounts WHERE id = ?1",
                [id.0.to_string()],
                |r| r.get(0),
            )
            .ok();
        match row {
            Some(json) => serde_json::from_str(&json).map(Some).map_err(|e| e.to_string()),
            None => Ok(None),
        }
    }

    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT json FROM accounts").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            let json = r.map_err(|e| e.to_string())?;
            // Skip rather than crash on a row that no longer decodes.
            if let Ok(acc) = serde_json::from_str(&json) {
                out.push(acc);
            }
        }
        Ok(out)
    }

    async fn upsert(&self, account: &ManagedAccount) -> Result<(), String> {
        let json = serde_json::to_string(account).map_err(|e| e.to_string())?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, json) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            (account.id.0.to_string(), json),
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn remove(&self, id: AccountId) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM accounts WHERE id = ?1", [id.0.to_string()])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Session cookie in the OS keychain. The entry value is the raw `Cookie:` header.
pub struct KeyringSessionStore;

#[async_trait]
impl SessionStore for KeyringSessionStore {
    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &user.0).map_err(|e| e.to_string())?;
        match entry.get_password() {
            Ok(cookie_header) => Ok(Some(Session {
                active_user: user.clone(),
                cookie_header,
            })),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn save(&self, session: &Session) -> Result<(), String> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &session.active_user.0)
            .map_err(|e| e.to_string())?;
        entry.set_password(&session.cookie_header).map_err(|e| e.to_string())
    }

    async fn delete(&self, user: &FramerUserId) -> Result<(), String> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &user.0).map_err(|e| e.to_string())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// File-based session store for headless servers/containers where no OS
/// keychain exists. One 0600 file per user under `dir`. Better than the
/// alternatives for a daemon box: not in the sqlite db, trivially deletable.
pub struct FileSessionStore {
    dir: std::path::PathBuf,
}

impl FileSessionStore {
    pub fn new(dir: impl Into<std::path::PathBuf>) -> Result<Self, String> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(Self { dir })
    }

    fn path(&self, user: &FramerUserId) -> std::path::PathBuf {
        self.dir.join(format!("{}.session", user.0))
    }
}

#[async_trait]
impl SessionStore for FileSessionStore {
    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        match std::fs::read_to_string(self.path(user)) {
            Ok(cookie_header) => Ok(Some(Session {
                active_user: user.clone(),
                cookie_header,
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn save(&self, session: &Session) -> Result<(), String> {
        let path = self.path(&session.active_user);
        std::fs::write(&path, &session.cookie_header).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    async fn delete(&self, user: &FramerUserId) -> Result<(), String> {
        match std::fs::remove_file(self.path(user)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}
