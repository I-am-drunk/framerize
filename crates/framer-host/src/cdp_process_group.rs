use std::{io, process::ExitStatus, sync::Arc};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::process::Child;

/// The group created for an owned browser, never an attached browser or probe.
pub(super) struct ProcessGroup {
    #[cfg(unix)]
    id: libc::pid_t,
    gone: AtomicBool,
}

impl ProcessGroup {
    /// Signal only while this unreaped leader still reserves its process ID.
    pub(super) fn request_kill(&self, child: &Child) {
        #[cfg(unix)]
        if child.id() == Some(self.id as u32) {
            // SAFETY: a positive child ID names our private group. Holding the
            // unreaped Child prevents this ID from being reused for another group.
            unsafe { libc::kill(-self.id, libc::SIGKILL); }
        }
        #[cfg(not(unix))]
        let _ = child;
    }

    pub(super) async fn wait_empty(&self) -> io::Result<()> {
        while !self.confirm_empty()? {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Ok(())
    }

    fn confirm_empty(&self) -> io::Result<bool> {
        if self.gone.load(Ordering::Acquire) { return Ok(true); }
        #[cfg(unix)]
        {
            // Signal zero observes existence without signaling any process.
            if unsafe { libc::kill(-self.id, 0) } == 0 { return Ok(false); }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) { return Err(error); }
        }
        self.gone.store(true, Ordering::Release);
        Ok(true)
    }
}

pub(super) async fn wait_for_exit(child: &mut Child, group: Option<&ProcessGroup>)
    -> io::Result<ExitStatus> {
    let status = child.wait().await?;
    if let Some(group) = group { group.wait_empty().await?; }
    Ok(status)
}

impl ProcessGroup {
    /// The caller spawned this child with process_group(0) on Unix.
    pub(super) fn for_child(child: &Child) -> Option<Arc<Self>> {
        #[cfg(unix)]
        {
            let id = child.id().expect("fresh browser child has no process ID");
            Some(Arc::new(Self { id: id as libc::pid_t, gone: AtomicBool::new(false) }))
        }
        #[cfg(not(unix))]
        { let _ = child; None }
    }
}
