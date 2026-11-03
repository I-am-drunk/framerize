use crate::{error::AccountCooldown, forward::ChainControl};
use std::{future::Future, time::Duration};
use tokio::time::Instant;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};

    #[derive(Default)]
    struct ChainConfig {
        cancelled: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    }

    impl ChainControl for ChainConfig {
        fn is_cancelled(&self) -> bool {
            self.cancelled.as_ref().is_some_and(|check| check())
        }
        fn on_429(&self) {}
    }

    #[tokio::test(start_paused = true)]
    async fn cooldown_never_retries_early_and_cancellation_stops_waiting() {
        for cancel in [false, true] {
            let flag = Arc::new(AtomicBool::new(false));
            let signal = flag.clone();
            let cfg = ChainConfig { cancelled:Some(Arc::new(move || signal.load(Ordering::SeqCst))),
                ..ChainConfig::default() };
            let calls = AtomicUsize::new(0);
            let mut wait = CooldownWait::default();
            let timer = wait.timer();
            {
                let future = wait.run(&cfg, true, || async {
                    if calls.fetch_add(1, Ordering::SeqCst) == 0 { Err("account-cooling:600".into()) }
                    else { Ok(()) }
                });
                tokio::pin!(future);
                assert!(futures::poll!(&mut future).is_pending());
                tokio::time::advance(Duration::from_secs(599)).await;
                assert!(futures::poll!(&mut future).is_pending());
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                flag.store(cancel, Ordering::SeqCst);
                let result = future.await;
                assert_eq!(result.is_err(), cancel);
                assert_eq!(calls.load(Ordering::SeqCst), if cancel { 1 } else { 2 });
            }
            assert_eq!(wait.elapsed(timer), Duration::ZERO);
            assert_eq!(wait.spent.as_secs(), if cancel { 599 } else { 600 });
        }
    }

    #[tokio::test(start_paused = true)]
    async fn cumulative_wait_is_bounded_and_generation_time_is_preserved() {
        let mut wait = CooldownWait::default();
        let timer = wait.timer();
        for (seconds, retries) in [(200, 2), (400, 2), (1, 1)] {
            let calls = AtomicUsize::new(0);
            let result = wait.run(&ChainConfig::default(), true, || async {
                tokio::time::sleep(Duration::from_secs(2)).await;
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(format!("account-cooling:{seconds}"))
                } else { Ok(()) }
            }).await;
            assert_eq!(result.is_ok(), retries == 2);
            assert_eq!(calls.load(Ordering::SeqCst), retries);
        }
        assert_eq!(wait.spent, Duration::from_secs(600));
        assert_eq!(wait.elapsed(timer), Duration::from_secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn a_renewed_cooldown_returns_without_repeated_waits() {
        let mut wait = CooldownWait::default();
        let calls = AtomicUsize::new(0);
        let result: Result<(), String> = wait.run(&ChainConfig::default(), true, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err("account-cooling:10".into())
        }).await;
        assert_eq!(result.unwrap_err(), "account-cooling:10");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(wait.spent, Duration::from_secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn first_chunk_and_nonlocal_errors_never_wait_or_retry() {
        for (has_output, error) in [(false, "account-cooling:10"),
            (true, "account-cooling:601"), (true, "risk-refused"),
            (true, "upstream 403: account-cooling:10"), (true, "account-cooling:bad")] {
            let mut wait = CooldownWait::default();
            let started = Instant::now();
            let calls = AtomicUsize::new(0);
            let result: Result<(), String> = wait.run(&ChainConfig::default(), has_output, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(error.into())
            }).await;
            assert_eq!(result.unwrap_err(), error);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(started.elapsed(), Duration::ZERO);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn guarded_waits_include_global_pauses_and_stop_at_the_total_bound() {
        let mut wait = CooldownWait::default();
        let active = crate::active_budget::ActiveBudget::new(&wait);
        let calls = AtomicUsize::new(0);
        let result: Result<(), String> = wait.run_guarded(&ChainConfig::default(), &active, 0, || async {
            let call = calls.fetch_add(1, Ordering::SeqCst);
            Err(match call {
                0 => "server-cooling:300", 1 => "account-cooling:300", _ => "account-cooling:1",
            }.into())
        }).await;
        assert_eq!(result.unwrap_err(), "account-cooling:1");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(wait.spent, Duration::from_secs(600));
        assert!(active.check(&wait, 0, None).is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn guarded_wait_cannot_reset_an_admission_budget() {
        let mut wait = CooldownWait::with_limit(Duration::from_secs(5));
        let active = crate::active_budget::ActiveBudget::new(&wait);
        let calls = AtomicUsize::new(0);
        let result: Result<(), String> = wait.run_guarded(&ChainConfig::default(), &active, 0, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err("account-cooling:3".into())
        }).await;
        assert_eq!(result.unwrap_err(), "account-cooling:3");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(wait.spent, Duration::from_secs(3));
    }
}

/// Preserve a partly delivered generation across a local capacity wait.
/// The default ten-minute budget can be replaced with remaining admission time.
pub struct CooldownWait {
    spent: Duration,
    limit: Duration,
}

impl Default for CooldownWait {
    fn default() -> Self { Self::with_limit(Duration::from_secs(600)) }
}

#[derive(Clone, Copy)]
pub struct Timer {
    started: Instant,
    waiting: Duration,
}

impl CooldownWait {
    pub fn with_limit(limit: Duration) -> Self { Self { spent: Duration::ZERO, limit } }

    async fn wait(&mut self, cfg: &(impl ChainControl + ?Sized), duration: Duration) -> Result<(), String> {
        let started = Instant::now();
        let deadline = started + duration;
        let result = loop {
            if let Err(error) = cfg.check_cancelled() {
                break Err(error);
            }
            if Instant::now() >= deadline {
                break Ok(());
            }
            tokio::time::sleep_until(deadline.min(Instant::now() + Duration::from_millis(100))).await;
        };
        self.spent += started.elapsed();
        result
    }

    pub fn timer(&self) -> Timer {
        Timer { started: Instant::now(), waiting: self.spent }
    }

    pub fn elapsed(&self, timer: Timer) -> Duration {
        timer.started.elapsed().saturating_sub(self.spent.saturating_sub(timer.waiting))
    }

    pub async fn run<T, F, Fut>(&mut self, cfg: &(impl ChainControl + ?Sized),
        has_output: bool, attempt: F) -> Result<T, String>
    where F: FnMut() -> Fut, Fut: Future<Output = Result<T, String>> {
        self.run_inner(cfg, has_output, false, None, attempt).await
    }

    pub async fn run_budgeted<T, F, Fut>(&mut self, cfg: &(impl ChainControl + ?Sized),
        has_output: bool, budget: &crate::active_budget::ActiveBudget,
        chunks: u32, attempt: F) -> Result<T, String>
    where F: FnMut() -> Fut, Fut: Future<Output = Result<T, String>> {
        self.run_inner(cfg, has_output, false, Some((budget, chunks)), attempt).await
    }

    /// A guarded request can wait before its first chunk and through renewed
    /// local cooldowns. All waits consume the configured cumulative budget.
    pub async fn run_guarded<T, F, Fut>(&mut self, cfg: &(impl ChainControl + ?Sized),
        budget: &crate::active_budget::ActiveBudget, chunks: u32, attempt: F)
        -> Result<T, String>
    where F: FnMut() -> Fut, Fut: Future<Output = Result<T, String>> {
        self.run_inner(cfg, true, true, Some((budget, chunks)), attempt).await
    }

    async fn run_inner<T, F, Fut>(&mut self, cfg: &(impl ChainControl + ?Sized),
        has_output: bool, repeat: bool, budget: Option<(&crate::active_budget::ActiveBudget, u32)>,
        mut attempt: F) -> Result<T, String>
    where F: FnMut() -> Fut, Fut: Future<Output = Result<T, String>> {
        let mut retried = false;
        loop {
            cfg.check_cancelled()?;
            if let Some((budget, chunks)) = budget { budget.check(self, chunks, None)?; }
            let error = match attempt().await {
                Ok(value) => return Ok(value),
                Err(error) => error,
            };
            let seconds = AccountCooldown::from_error(&error).map(|c| c.retry_after_secs)
                .or_else(|| if repeat {
                    crate::error::chain_error_cause(&error).strip_prefix("server-cooling:")
                        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                        .and_then(|s| s.parse::<u64>().ok())
                } else { None });
            let Some(seconds) = seconds.filter(|_| has_output && (!retried || repeat)) else {
                return Err(error);
            };
            let wait = Duration::from_secs(seconds.max(1));
            if wait > self.limit.saturating_sub(self.spent) {
                return Err(error);
            }
            if let Some((budget, chunks)) = budget { budget.check(self, chunks, Some(&error))?; }
            self.wait(cfg, wait).await?;
            // The caller retains identical input across each acknowledged refusal.
            cfg.check_cancelled()?;
            if let Some((budget, chunks)) = budget { budget.check(self, chunks, Some(&error))?; }
            retried = true;
        }
    }
}
