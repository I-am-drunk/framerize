use crate::{cooldown_wait::{CooldownWait, Timer}, forward::ChainControl};
use std::time::Duration;
use tokio::time::Instant;

const LIMIT: Duration = Duration::from_secs(300);

pub struct ActiveBudget {
    started: Timer,
}

impl ActiveBudget {
    pub async fn pause(&self, wait: &CooldownWait, control: &(impl ChainControl + ?Sized),
        duration: Duration, chunks: u32, cause: Option<&str>) -> Result<(), String>
    {
        let deadline = Instant::now() + duration;
        loop {
            control.check_cancelled()?;
            self.check(wait, chunks, cause)?;
            let now = Instant::now();
            if now >= deadline { return Ok(()); }
            tokio::time::sleep_until(deadline.min(now + Duration::from_millis(100))).await;
        }
    }

    pub fn new(wait: &CooldownWait) -> Self {
        Self { started: wait.timer() }
    }

    pub fn check(&self, wait: &CooldownWait, chunks: u32, cause: Option<&str>)
        -> Result<(), String>
    {
        if wait.elapsed(self.started) < LIMIT { return Ok(()); }
        let seconds = LIMIT.as_secs();
        let mut error = format!("chain exceeded {seconds}s active-time budget after {chunks} chunks (excluding local cooldown waits), retry the request");
        if let Some(cause) = cause {
            error.push_str("; last failure: ");
            error.push_str(&crate::error::safe_failure_summary(cause));
        }
        Err(error)
    }
}

#[cfg(test)]
#[path = "active_budget_tests.rs"]
mod tests;
