//! Keep profile ownership until the optional migration process has exited.
use framer_core::port::SessionStore;
use futures::future::{BoxFuture, Shared};
use std::{path::PathBuf, process::Stdio, sync::Arc};
use tokio::{process::Command, time::{timeout, Duration}};

pub(super) async fn expire(python: PathBuf, script: PathBuf, root: PathBuf,
    previous: PathBuf, sessions: Arc<dyn SessionStore>, stop: Shared<BoxFuture<'static, ()>>)
    -> Result<bool, String> {
    let previous = previous.canonicalize().map_err(|_| "previous runtime metadata is missing")?;
    let (mut sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _sessions = sessions;
        let result = async {
            let mut child = Command::new(python).arg("-B").arg(script)
                .arg("--root").arg(&root).arg("--expire-legacy-from").arg(previous)
                .env_clear().env("PATH", "/usr/bin:/bin").process_group(0)
                .envs(super::super::account::ledger::rate_policy_environment())
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::inherit())
                .kill_on_drop(true).spawn().map_err(|_| "cannot start legacy migration")?;
            let result = tokio::select! {
                biased;
                _ = stop => Ok(false),
                _ = sender.closed() => Ok(false),
                result = timeout(Duration::from_secs(660), child.wait()) => match result {
                    Ok(Ok(status)) if status.success() => Ok(true),
                    _ => Err("legacy migration failed or timed out".to_string()),
                },
            };
            if !matches!(result, Ok(true)) {
                if let Some(pid) = child.id() {
                    // The unreaped child still owns this PID. Ask Python to
                    // release its witness before resorting to forced cleanup.
                    unsafe { libc::kill(pid as i32, libc::SIGTERM); }
                }
                if !matches!(timeout(Duration::from_secs(5), child.wait()).await, Ok(Ok(_))) {
                    let _ = child.start_kill();
                    child.wait().await.map_err(|_| "migration cleanup is unconfirmed")?;
                }
            }
            let completed = result?;
            if completed && !root.join("rate-history.sqlite3").is_file() {
                return Err("migration did not publish account history".to_string());
            }
            Ok(completed)
        }.await;
        drop(_sessions);
        let _ = sender.send(result);
    });
    receiver.await.map_err(|_| "migration supervisor is unavailable")?
}
