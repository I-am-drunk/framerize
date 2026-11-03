//! Structured observability — one canonical event per operation.
//!
//! Design (docs/logging.md, follows the structured-logging checklist):
//! - NDJSON, one event per line, at `<profiles>/.logs/events-YYYY-MM-DD.ndjson`
//!   (shared with the sidecar, which gets the dir via FRAMERIZE_LOG_DIR).
//! - Every event carries standard fields (OTel-ish): ts, level, event
//!   (dot-namespaced), request_id, account, duration_ms, outcome — plus
//!   event-specific fields.
//! - Canonical wide events (one per operation with all phases), not a spray
//!   of step logs: `login.begin`, `login.complete`, `ai.request`,
//!   `ai.chain`, `castle.mint`, `account.import`, `daemon.start`,
//!   `credits.refresh`, `sidecar.respawn`.
//! - Credit accounting is first-class: `credits_before` / `credits_after` /
//!   `credits_delta` / `metered_chunks` on AI events, so non-consumption of
//!   the free path is *provable* from the log.
//! - NEVER log secrets: no castle tokens, session cookies, JWTs, prompt
//!   text. Lengths and kinds are fine; values are not. Emails are masked.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[path = "observability_append.rs"]
mod append;

#[path = "observability_daily.rs"]
mod daily;

static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();
static WRITER: OnceLock<Mutex<daily::DailyWriter>> = OnceLock::new();
static LOG_WRITE_WARNING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn warn_write_failure(error: &std::io::Error) {
    use std::io::Write;
    use std::sync::atomic::Ordering;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_secs();
    let previous = LOG_WRITE_WARNING.load(Ordering::Relaxed);
    if previous != 0 && now.saturating_sub(previous) < 60 { return; }
    if LOG_WRITE_WARNING.compare_exchange(previous, now, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
        let _ = writeln!(std::io::stderr().lock(),
            "Framerize event log write failed: {error}; structured diagnostics are being lost.");
    }
}

pub fn init(profiles: &str) {
    init_path(Path::new(profiles));
}

pub fn init_path(profiles: &Path) {
    let dir = profiles.join(".logs");
    let _ = std::fs::create_dir_all(&dir);
    let _ = LOG_DIR.set(dir);
    if let Some(dir) = LOG_DIR.get() {
        daily::prune(dir, &chrono_today());
    }
}

fn file_for_today() -> Option<&'static Mutex<daily::DailyWriter>> {
    // Never panic for logging: uninitialized → no writer → emit() is a no-op.
    // (Library callers like convert::to_openrouter_id run in tests without init.)
    LOG_DIR.get()?;
    Some(WRITER.get_or_init(|| Mutex::new(daily::DailyWriter::default())))
}

fn chrono_today() -> String {
    // YYYY-MM-DD without pulling in chrono.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = secs / 86400;
    let (y, m, d) = days_to_ymd(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn days_to_ymd(days: u64) -> (i64, u32, u32) {
    // civil-from-days (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn now_iso() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = t.as_secs();
    let days = secs / 86400;
    let tod = secs % 86400;
    let (y, m, d) = days_to_ymd(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60,
        t.subsec_millis()
    )
}

pub struct Event {
    v: Value,
}

impl Event {
    pub fn new(event: &str) -> Self {
        Self {
            v: json!({
                "ts": now_iso(),
                "level": "info",
                "event": event,
                "request_id": uuid::Uuid::new_v4().to_string(),
            }),
        }
    }
    pub fn level(mut self, level: &str) -> Self {
        self.v["level"] = json!(level);
        self
    }
    pub fn request_id(mut self, id: &str) -> Self {
        self.v["request_id"] = json!(id);
        self
    }
    pub fn account(mut self, id: &crate::ids::AccountId) -> Self {
        let s = id.0.to_string();
        self.v["account"] = json!(&s[..8.min(s.len())]);
        self
    }
    pub fn field(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.v[key] = value.into();
        self
    }
    pub fn duration_ms(mut self, ms: u128) -> Self {
        self.v["duration_ms"] = json!(ms);
        self
    }
    pub fn outcome(mut self, outcome: &str) -> Self {
        self.v["outcome"] = json!(outcome);
        self
    }
    pub fn error(self, msg: &str) -> Self {
        self.level("error").field("error", msg.to_string()).outcome("error")
    }
    pub fn emit(self) {
        if let (Some(f), Some(directory)) = (file_for_today(), LOG_DIR.get()) {
            if let Ok(mut f) = f.lock() {
                if let Err(error) = f.event(directory, &chrono_today(), &self.v) {
                    warn_write_failure(&error);
                }
            }
        }
    }
}

/// Mask an email for logs: `jacobkeldo@gmail.com` → `j***@gmail.com`.
pub fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) if !local.is_empty() => format!("{}***@{}", &local[..1], domain),
        _ => "***".into(),
    }
}

/// Milliseconds since an instant, for duration fields.
pub fn elapsed_ms(t: std::time::Instant) -> u128 {
    t.elapsed().as_millis()
}
