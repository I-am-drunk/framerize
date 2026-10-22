use super::*;

pub(crate) enum ProcessMode {
    Unmanaged,
    Observed { script: PathBuf, profile_nonce: Uuid },
}

impl SidecarBridge {
    /// Allocate a new isolated profile root without starting a process.
    /// Never adopts or copies live profiles. Credentials are supplied later.
    pub fn new_observed_one_shot(
        state_parent: impl AsRef<Path>, script: impl AsRef<Path>,
    ) -> Result<Self, ObserverError> {
        let parent = std::fs::canonicalize(state_parent).map_err(|_| ObserverError::Profile)?;
        let script = std::fs::canonicalize(script).map_err(|_| ObserverError::Profile)?;
        if !parent.is_dir() || !script.is_file()
            || !script.parent().unwrap().join("observed_sidecar.py").is_file() {
            return Err(ObserverError::Profile);
        }
        let nonce = Uuid::new_v4();
        let root = parent.join(format!("observed-{nonce}"));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)] {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&root).map_err(|_| ObserverError::Profile)?;
        builder.create(root.join("camoufox")).map_err(|_| ObserverError::Profile)?;
        std::fs::write(root.join("camoufox/.observer-owner"), nonce.to_string())
            .map_err(|_| ObserverError::Profile)?;
        let mut bridge = Self::new(root);
        bridge.mode = ProcessMode::Observed { script, profile_nonce: nonce };
        Ok(bridge)
    }
}
