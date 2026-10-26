use std::{fs::{File, OpenOptions}, io::{self, Write}, path::{Path, PathBuf}};

pub(super) async fn create(path: PathBuf, contents: Vec<u8>) -> io::Result<String> {
    create_with(path, contents, |file, bytes| file.write_all(bytes)).await
}

async fn create_with<F>(path: PathBuf, contents: Vec<u8>, mut write: F) -> io::Result<String>
where F: FnMut(&mut File, &[u8]) -> io::Result<()> + Send + 'static {
    // The blocking owner retains both the candidate and its cleanup even if
    // the async caller disappears. It never truncates the published record.
    tokio::task::spawn_blocking(move || publish(&path, &contents, &mut write))
        .await.map_err(|_| io::Error::new(io::ErrorKind::Other,
            "browser device publication task failed"))?
}

struct Candidate {
    path: PathBuf,
    file: Option<File>,
    removed: bool,
}

fn publish<F>(path: &Path, contents: &[u8], write: &mut F) -> io::Result<String>
where F: FnMut(&mut File, &[u8]) -> io::Result<()> {
    let mut candidate = Candidate::new(path)?;
    let result = (|| {
        let file = candidate.file.as_mut().unwrap();
        write(file, contents)?;
        file.sync_all()?;
        drop(candidate.file.take());
        // A hard link publishes the complete inode atomically and refuses
        // replacement. A concurrent winner remains byte-for-byte unchanged.
        match std::fs::hard_link(&candidate.path, path) {
            Ok(()) => {},
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {},
            Err(error) => return Err(error),
        }
        std::fs::read_to_string(path)
    })();
    let removed = candidate.remove();
    match result { Ok(raw) => { removed?; Ok(raw) }, Err(error) => Err(error) }
}

#[cfg(test)]
#[path = "cdp_device_publication_tests.rs"]
mod tests;

impl Drop for Candidate {
    fn drop(&mut self) { let _ = self.remove(); }
}

impl Candidate {
    fn new(target: &Path) -> io::Result<Self> {
        let parent = target.parent().ok_or_else(||
            io::Error::new(io::ErrorKind::InvalidInput, "device record has no parent"))?;
        let path = parent.join(format!(".framerize-device-{}.tmp", uuid::Uuid::new_v4()));
        let file = OpenOptions::new().write(true).create_new(true).open(&path)?;
        Ok(Self { path, file: Some(file), removed: false })
    }

    fn remove(&mut self) -> io::Result<()> {
        drop(self.file.take());
        if self.removed { return Ok(()); }
        match std::fs::remove_file(&self.path) {
            Ok(()) => {},
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
        self.removed = true;
        Ok(())
    }
}
