use std::{fs, path::Path};

pub(super) fn prepare(directory: &Path) -> Result<(), String> {
    let fail = |e| format!("Claude checkpoint history storage: {e}");
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)] {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(directory) {
        Ok(()) => {},
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {},
        Err(e) => return Err(fail(e)),
    }
    let info = fs::symlink_metadata(directory).map_err(fail)?;
    if !info.is_dir() || info.file_type().is_symlink() {
        return Err("Claude checkpoint history directory is not a real directory".into());
    }
    let path = directory.join("history.sqlite3");
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(_) => {},
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {},
        Err(e) => return Err(fail(e)),
    }
    let file = fs::symlink_metadata(path).map_err(fail)?;
    if !file.is_file() || file.file_type().is_symlink() {
        return Err("Claude checkpoint history must be a regular file".into());
    }
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        if info.permissions().mode() & 0o077 != 0 || file.permissions().mode() & 0o077 != 0 {
            return Err("Claude checkpoint history directory and file must be private".into());
        }
    }
    Ok(())
}
