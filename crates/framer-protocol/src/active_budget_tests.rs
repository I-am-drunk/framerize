use super::*;

struct Control(Option<Instant>);
impl ChainControl for Control {
    fn is_cancelled(&self) -> bool { self.0.is_some_and(|at| Instant::now() >= at) }
    fn on_429(&self) {}
}

#[tokio::test(start_paused = true)]
async fn deadline_interrupts_backoff_and_preserves_its_cause() {
    let wait = CooldownWait::default();
    let budget = ActiveBudget::new(&wait);
    tokio::time::advance(Duration::from_secs(299)).await;
    let started = Instant::now();
    let result = budget.pause(&wait, &Control(None), Duration::from_secs(24), 0,
        Some("retryable 429: provider queue full")).await;
    let error = result.unwrap_err();
    assert!(error.contains("300s active-time budget"));
    assert!(error.contains("retryable 429: provider queue full"));
    assert!(started.elapsed() <= Duration::from_millis(1100));
}

#[tokio::test(start_paused = true)]
async fn no_new_attempt_is_admitted_at_the_exact_deadline() {
    let wait = CooldownWait::default();
    let budget = ActiveBudget::new(&wait);
    tokio::time::advance(Duration::from_secs(300)).await;
    assert!(budget.check(&wait, 0, None).is_err());
}

#[tokio::test(start_paused = true)]
async fn client_cancellation_interrupts_a_long_rate_backoff() {
    let wait = CooldownWait::default();
    let budget = ActiveBudget::new(&wait);
    let started = Instant::now();
    let control = Control(Some(started + Duration::from_millis(100)));
    let result = budget.pause(&wait, &control, Duration::from_secs(24), 0, None).await;
    assert_eq!(result.unwrap_err(), "client cancelled");
    assert!(started.elapsed() < Duration::from_secs(1));
}
