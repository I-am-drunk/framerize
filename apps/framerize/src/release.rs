//! Assets belong to the selected executable, independently of the current directory.
use std::path::{Path, PathBuf};

pub fn web_dir(executable: &Path, override_path: Option<PathBuf>) -> Result<PathBuf, String> {
    let path = match override_path {
        Some(path) => path,
        None => executable.canonicalize().map_err(|e| e.to_string())?
            .parent().and_then(Path::parent)
            .ok_or("cannot resolve application package")?.join("web"),
    };
    if !path.join("index.html").is_file() {
        return Err("web UI missing; install the full archive or pass --web-dir for development".into());
    }
    path.canonicalize().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_selected_release_and_checks_development_override() {
        let root = std::env::temp_dir().join(format!("framerize-ui-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let binary = root.join("bin/framerize");
        std::fs::write(&binary, "fixture").unwrap();
        assert!(web_dir(&binary, None).is_err());
        std::fs::create_dir(root.join("web")).unwrap();
        std::fs::write(root.join("web/index.html"), "fixture").unwrap();
        assert_eq!(web_dir(&binary, None).unwrap(), root.join("web"));
        assert!(web_dir(&binary, Some(root.join("absent"))).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&binary, root.join("current")).unwrap();
            assert_eq!(web_dir(&root.join("current"), None).unwrap(), root.join("web"));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
