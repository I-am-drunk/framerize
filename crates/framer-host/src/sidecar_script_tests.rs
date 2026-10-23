use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("framerize-host-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
    fn script(&self) -> PathBuf {
        let path = self.0.join("host.py");
        std::fs::write(&path, "# isolated fixture, never executed\n").unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

#[test]
fn explicit_script_selects_only_the_instance_host_without_writing_it() {
    let fixture = Fixture::new();
    let script = fixture.script();
    let before = std::fs::read(&script).unwrap();
    let bridge = SidecarBridge::new(fixture.0.join("profiles/.sidecar"))
        .with_script(&script).unwrap();
    assert_eq!(bridge.script_path(), script);
    assert_eq!(std::fs::read(&script).unwrap(), before);
    assert!(!bridge.state_root.join(".bin").exists());
    assert!(bridge.inner.try_lock().unwrap().is_none());
}

#[test]
fn explicit_script_rejects_noncanonical_or_invalid_files() {
    let fixture = Fixture::new();
    let script = fixture.script();
    let empty = fixture.0.join("empty.py");
    std::fs::write(&empty, "").unwrap();
    let large = fixture.0.join("large.py");
    std::fs::File::create(&large).unwrap().set_len(4 * 1024 * 1024 + 1).unwrap();
    for path in [PathBuf::from("relative.py"), fixture.0.join("missing.py"),
        fixture.0.clone(), empty, large] {
        assert!(SidecarBridge::new(&fixture.0).with_script(path).is_err());
    }
    #[cfg(unix)]
    {
        let link = fixture.0.join("link.py");
        std::os::unix::fs::symlink(&script, &link).unwrap();
        assert!(SidecarBridge::new(&fixture.0).with_script(link).is_err());
    }
}
