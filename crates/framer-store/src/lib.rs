//! Persistence, split by sensitivity (ADR-001):
//!   - `SqliteStateStore` holds non-secret account state. It has NO secret columns.
//!   - `KeyringSessionStore` holds the `session` cookie in the OS keychain, keyed
//!     by Framer user id.

use async_trait::async_trait;
use framer_core::account::ManagedAccount;
use framer_core::ids::{AccountId, FramerUserId};
use framer_core::port::{Session, SessionStore, StateStore};
use rusqlite::{Connection, OptionalExtension};
use std::sync::Mutex;

const KEYCHAIN_SERVICE: &str = "framerize.session";

mod file_sessions;
mod legacy_device_ids;
pub use file_sessions::FileSessionStore;

#[cfg(test)]
mod sqlite_tests;

#[cfg(test)]
mod sqlite_list_tests;

#[cfg(test)]
mod legacy_device_id_tests;

pub struct SqliteStateStore {
    conn: Mutex<Connection>,
}

impl SqliteStateStore {
    pub fn open(path: &str) -> Result<Self, String> {
        let mut conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS accounts (
                id            TEXT PRIMARY KEY,
                json          TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );",
        )
        .map_err(|e| e.to_string())?;
        legacy_device_ids::persist(&mut conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }
}

#[async_trait]
impl StateStore for SqliteStateStore {
    async fn setting(&self, key: &str) -> Result<Option<String>, String> {
        self.conn.lock().map_err(|e| e.to_string())?
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| row.get(0))
            .optional().map_err(|e| e.to_string())
    }

    async fn save_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn.lock().map_err(|e| e.to_string())?.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value", (key, value),
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        let conn = self.conn.lock().unwrap();
        let row: Option<String> = conn
            .query_row(
                "SELECT json FROM accounts WHERE id = ?1",
                [id.0.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
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
            let account = serde_json::from_str(&json)
                .map_err(|_| "stored account could not be decoded".to_string())?;
            out.push(account);
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
