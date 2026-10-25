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

#[tokio::test]
async fn browser_bridge_refuses_native_startup_without_touching_state() {
    use framer_core::{ids::FramerUserId, native_device::DeviceFailure};
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let bridge = SidecarBridge::new(&state);
    let account = AccountId(uuid::Uuid::new_v4());
    assert!(matches!(bridge.open_native_device(account).await, Err(DeviceFailure::Rejected)));
    assert!(matches!(bridge.enroll_native_device(account,
        &FramerUserId("fixture".into()), "fixture-device").await,
        Err(DeviceFailure::Rejected)));
    assert!(bridge.inner.try_lock().unwrap().is_none());
    assert!(!state.exists());
}

#[test]
fn explicit_script_selects_only_the_instance_host_without_writing_it() {
    let fixture = Fixture::new();
    let script = fixture.script();
    let before = std::fs::read(&script).unwrap();
    let bridge = SidecarBridge::new(fixture.0.join("profiles/.sidecar"))
        .with_script(&script).unwrap();
    assert_eq!(bridge.script_path().unwrap(), script);
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

#[test]
fn package_follows_the_selected_executable_and_requires_runtime_files() {
    let fixture = Fixture::new();
    let binary = fixture.0.join("release/bin/framerize");
    std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
    std::fs::write(&binary, "fixture").unwrap();
    assert!(bundled_host(&binary).is_err());
    let runtime = fixture.0.join("release/runtime");
    std::fs::create_dir_all(runtime.join("framerize_host")).unwrap();
    for name in ["host.py", "runtime-manifest.json", "framerize_host/__main__.py", "framerize_host/app.py",
        "framerize_host/castle_forge.py"] {
        std::fs::write(runtime.join(name), "# fixture\n").unwrap();
    }
    assert_eq!(bundled_host(&binary).unwrap(), runtime.join("host.py"));
    #[cfg(unix)]
    {
        let selected = fixture.0.join("current");
        std::os::unix::fs::symlink(&binary, &selected).unwrap();
        assert_eq!(bundled_host(&selected).unwrap(), runtime.join("host.py"));
    }
    std::fs::remove_file(runtime.join("framerize_host/castle_forge.py")).unwrap();
    assert!(bundled_host(&binary).is_err());
}
