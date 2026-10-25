//! Application-selected storage for learned concurrency limits.
use std::{collections::HashMap, fs, io::{self, Write}, path::Path};

pub(super) fn load(path: &Path) -> io::Result<HashMap<String, usize>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(error) => Err(error),
    }
}

pub(super) fn save(path: &Path, limits: &HashMap<String, usize>) -> io::Result<()> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".gate-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec(limits)?)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(temporary); }
    result
}
