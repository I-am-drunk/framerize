use serde_json::Value;
use std::{fs::{File, OpenOptions}, io, path::Path};

#[cfg(test)]
#[path = "observability_daily_tests.rs"]
mod tests;

#[derive(Default)]
pub(super) struct DailyWriter { current: Option<(String, File)> }

impl DailyWriter {
    pub(super) fn event(&mut self, directory: &Path, day: &str, value: &Value) -> io::Result<()> {
        if self.current.as_ref().is_none_or(|(opened, _)| opened != day) {
            let path = directory.join(format!("events-{day}.ndjson"));
            let file = OpenOptions::new().create(true).append(true).open(&path)?;
            self.current = Some((day.to_owned(), file));
            prune(directory, day);
        }
        super::append::event(&mut self.current.as_mut().unwrap().1, value)
    }
}

pub(super) fn prune(directory: &Path, current_day: &str) {
    let current = format!("events-{current_day}.ndjson");
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(7 * 86400);
    let Ok(entries) = std::fs::read_dir(directory) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue; };
        let Some(date) = name.strip_prefix("events-").and_then(|s| s.strip_suffix(".ndjson"))
            else { continue; };
        let daily_name = date.len() == 10 && date.bytes().enumerate().all(|(i, b)|
            if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
        if !daily_name || name == current { continue; }
        if let Ok(metadata) = entry.path().symlink_metadata() {
            if metadata.is_file() && metadata.modified().is_ok_and(|time| time < cutoff) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}
