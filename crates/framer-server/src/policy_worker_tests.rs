use super::*;

mod invocation_contract {
    use crate::policy_worker_request::InvocationLedger;
    use framer_core::port::{ForwardError, OutputError, ResponseError};
    use framer_worker_wire::message::{self as wire, Message};
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

    fn cause(fwd: u64) -> wire::Cause { wire::Cause { op: 7, n: 2, fwd } }
    fn fail(ledger: &mut InvocationLedger, fwd: u64, raw: &str, http: bool) {
        ledger.accept_forward(cause(fwd), 0).unwrap();
        let error = ForwardError::from_legacy(raw.into());
        ledger.forward_finished(cause(fwd), Some(if http { error.http() } else { error })).unwrap();
    }
    fn outcome(fwd: u64, tag: wire::FailureTag) -> wire::Outcome {
        wire::Outcome::Forward { failure: wire::FailureRecipe { cause: cause(fwd), tag, chunk: 0 } }
    }
    fn summary() -> wire::Outcome {
        wire::Outcome::Success { summary: wire::Summary { input_tokens: 2, output_tokens: 3,
            chunks: 1, metered_chunks: 0, finish: wire::FinishReason::Completed } }
    }
    #[test]
    fn saturation_uses_captured_effect_before_ack_and_rejects_overtaking() {
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = calls.clone();
        let mut ledger = InvocationLedger::new(7, 2, 4,
            Some(Arc::new(move || { captured.fetch_add(1, Ordering::SeqCst); }))).unwrap();
        fail(&mut ledger, 1, "retryable 429 on /responses: {}", true);
        assert_eq!(ledger.saturated(cause(1)).unwrap(), Message::SaturatedAck { op: 7, n: 2, fwd: 1 });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!ledger.settled());
        assert!(ledger.saturated(cause(1)).is_err());
        assert!(ledger.accept_forward(cause(2), 0).is_err());
        ledger.acknowledged(cause(1)).unwrap();
        assert!(ledger.saturated(cause(1)).is_err());
        fail(&mut ledger, 2, "retryable 429", true);
        ledger.saturated(cause(2)).unwrap();
        drop(ledger); // Losing an Ack waiter cannot undo the already applied effect.
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let next_team = Arc::new(AtomicUsize::new(0));
        let next = next_team.clone();
        let mut repicked = InvocationLedger::new(7, 3, 4,
            Some(Arc::new(move || { next.fetch_add(1, Ordering::SeqCst); }))).unwrap();
        assert!(repicked.saturated(cause(2)).is_err());
        assert_eq!(next_team.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn result_renders_exhaustion_and_fallback_before_classifying_once() {
        let raw = r#"retryable 429 on /responses: {"error":{"message":"busy","type":"limit_error"}}"#;
        let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
        for fwd in 1..=9 {
            fail(&mut ledger, fwd, raw, true);
            ledger.saturated(cause(fwd)).unwrap();
            ledger.acknowledged(cause(fwd)).unwrap();
        }
        fail(&mut ledger, 10, raw, true);
        assert!(ledger.saturated(cause(10)).is_err());
        ledger.close_result(7, 2).unwrap();
        let result = ledger.resolve(outcome(10, wire::FailureTag::SaturationExhausted), false).unwrap();
        let public = result.public_failure.unwrap();
        assert_eq!((public.status, public.error_type.as_str()), (500, "api_error"));
        assert_eq!(public.message.as_str(), format!("chunk 0: workspace still saturated after 9 retries ({raw})"));
        let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
        fail(&mut ledger, 1, raw, true);
        ledger.close_result(7, 2).unwrap();
        let result = ledger.resolve(outcome(1, wire::FailureTag::FallbackOff), false).unwrap();
        let public = result.public_failure.unwrap();
        assert_eq!((public.status, public.error_type.as_str()), (429, "limit_error"));
        assert_eq!(public.message.as_str(), "busy");
    }
    #[test]
    fn local_display_is_fixed_after_legacy_overflow_and_risk_classification() {
        for (raw, status, code) in [
            ("private cause: maximum context length", 500, "context_length_exceeded"),
            ("private risk-refused cause", 403, "api_error"),
            ("private transport cause", 500, "api_error"),
        ] {
            let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
            fail(&mut ledger, 1, raw, false);
            ledger.close_result(7, 2).unwrap();
            let result = ledger.resolve(outcome(1, wire::FailureTag::Direct), false).unwrap();
            let public = result.public_failure.unwrap();
            assert_eq!(public.status, status);
            assert_eq!(public.code.as_str(), code);
            assert_eq!(public.message.as_str(), "upstream operation failed");
            assert!(matches!(result.result, Err(ResponseError::Upstream(message)) if message == raw));
        }
    }
    #[test]
    fn result_barrier_refuses_resolution_until_forward_verdict_and_closes_dispatch() {
        let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
        ledger.accept_forward(cause(1), 0).unwrap();
        ledger.close_result(7, 2).unwrap();
        assert!(!ledger.settled());
        assert!(ledger.accept_forward(cause(2), 0).is_err());
        assert!(ledger.close_result(7, 2).is_err());
        ledger.forward_finished(cause(1), None).unwrap();
        assert!(ledger.settled());
        assert!(ledger.resolve(summary(), false).unwrap().result.is_ok());
        let mut pending = InvocationLedger::new(7, 2, 4, None).unwrap();
        pending.accept_forward(cause(1), 0).unwrap();
        pending.close_result(7, 2).unwrap();
        assert!(pending.resolve(summary(), false).is_err());
    }
    #[test]
    fn result_recipe_rejects_foreign_unknown_and_out_of_range_causes() {
        for recipe in [
            wire::FailureRecipe { cause: wire::Cause { op: 8, ..cause(1) }, tag: wire::FailureTag::Direct, chunk: 0 },
            wire::FailureRecipe { cause: wire::Cause { n: 1, ..cause(1) }, tag: wire::FailureTag::Direct, chunk: 0 },
            wire::FailureRecipe { cause: cause(2), tag: wire::FailureTag::Direct, chunk: 0 },
            wire::FailureRecipe { cause: cause(1), tag: wire::FailureTag::Direct, chunk: 4 },
            wire::FailureRecipe { cause: cause(1), tag: wire::FailureTag::SaturationExhausted, chunk: 0 },
        ] {
            let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
            fail(&mut ledger, 1, "retryable 429", true);
            ledger.close_result(7, 2).unwrap();
            assert!(ledger.resolve(wire::Outcome::Forward { failure: recipe }, false).is_err());
        }
        let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
        ledger.accept_forward(cause(1), 0).unwrap();
        ledger.forward_finished(cause(1), None).unwrap();
        ledger.close_result(7, 2).unwrap();
        assert!(ledger.resolve(outcome(1, wire::FailureTag::Direct), false).is_err());
    }
    #[test]
    fn disconnected_completion_needs_last_joined_success_and_preserves_encoding_errors() {
        for (success, cancelled, failure, expected) in [
            (true, true, wire::OutputFailure::Closed, true),
            (true, true, wire::OutputFailure::Cancelled, true),
            (true, false, wire::OutputFailure::Closed, false),
            (true, true, wire::OutputFailure::Encoding, false),
            (false, true, wire::OutputFailure::Closed, false),
        ] {
            let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
            ledger.accept_forward(cause(1), 0).unwrap();
            ledger.forward_finished(cause(1), (!success).then(|| ForwardError::from_legacy("failed".into()))).unwrap();
            ledger.close_result(7, 2).unwrap();
            let decision = ledger.resolve(wire::Outcome::Output { failure, completed: true }, cancelled).unwrap();
            assert_eq!(decision.completed_on_disconnect, expected);
            if failure == wire::OutputFailure::Encoding {
                assert!(matches!(decision.result, Err(ResponseError::Output(OutputError::Encoding))));
            }
        }
    }
    #[test]
    fn control_failure_never_becomes_retryable_display() {
        let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
        ledger.close_result(7, 2).unwrap();
        assert!(ledger.accept_forward(cause(1), 0).is_err());
        assert!(ledger.resolve(wire::Outcome::Control, false).is_err());
    }
    #[test]
    fn joined_cleanup_uncertainty_survives_a_later_success() {
        let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
        fail(&mut ledger, 1, "host-lifecycle-unconfirmed: fixture", false);
        assert!(!ledger.cleanup_confirmed());
        ledger.accept_forward(cause(2), 0).unwrap();
        ledger.forward_finished(cause(2), None).unwrap();
        assert!(!ledger.cleanup_confirmed());
        ledger.close_result(7, 2).unwrap();
        assert!(ledger.resolve(summary(), false).unwrap().result.is_ok());
    }
    #[test]
    fn absent_saturation_callback_is_legal_but_a_stale_429_is_not() {
        let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
        fail(&mut ledger, 1, "retryable 429", true);
        ledger.saturated(cause(1)).unwrap();
        ledger.acknowledged(cause(1)).unwrap();
        fail(&mut ledger, 2, "retryable 429", true);
        ledger.accept_forward(cause(3), 0).unwrap();
        ledger.forward_finished(cause(3), None).unwrap();
        assert!(ledger.saturated(cause(2)).is_err());
    }
}

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("framer-worker-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn executable(&self, bytes: &[u8]) -> Executable {
        let path = self.0.join("worker");
        std::fs::write(&path, bytes).unwrap();
        Executable { path, sha256: Sha256::digest(bytes).into() }
    }
}
impl Drop for Scratch {
    fn drop(&mut self) { std::fs::remove_dir_all(&self.0).unwrap(); }
}

#[cfg(target_os = "linux")]
#[test]
fn executable_verification_rejects_mismatch_non_elf_and_oversize() {
    let scratch = Scratch::new();
    let mut descriptor = scratch.executable(b"\x7fELF-synthetic");
    descriptor.sha256 = [0; 32];
    assert!(matches!(verify(&descriptor), Err(ProcessError::Executable)));
    let descriptor = scratch.executable(b"#!/bin/sh\n");
    assert!(matches!(verify(&descriptor), Err(ProcessError::Executable)));
    let descriptor = scratch.executable(b"\x7fELF-synthetic");
    std::fs::OpenOptions::new().write(true).open(&descriptor.path).unwrap()
        .set_len(MAX_EXECUTABLE_BYTES + 1).unwrap();
    assert!(matches!(verify(&descriptor), Err(ProcessError::Executable)));
    let relative = Executable { path: "worker".into(), sha256: [0; 32] };
    assert!(matches!(verify(&relative), Err(ProcessError::Executable)));
}

#[cfg(target_os = "linux")]
#[test]
fn opened_executable_keeps_the_hashed_inode_after_path_replacement() {
    use std::io::{Seek, SeekFrom};
    use std::os::fd::AsRawFd;
    let scratch = Scratch::new();
    let original = b"\x7fELF-original-synthetic-inode";
    let executable = scratch.executable(original);
    let mut file = verify(&executable).unwrap();
    assert!(file.as_raw_fd() > 2);
    std::fs::rename(&executable.path, scratch.0.join("old")).unwrap();
    std::fs::write(&executable.path, b"\x7fELF-replacement").unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut retained = Vec::new();
    file.read_to_end(&mut retained).unwrap();
    assert_eq!(retained, original);
    assert!(verify(&executable).is_err());
    let command = command(&file);
    assert_eq!(command.get_program(), std::ffi::OsStr::new(
        &format!("/proc/self/fd/{}", file.as_raw_fd())));
    assert_eq!(command.get_current_dir(), Some(std::path::Path::new("/")));
    assert_eq!(command.get_args().count(), 0);
}

#[tokio::test]
async fn stderr_is_counted_and_discarded_without_retaining_payloads() {
    let counts = Arc::new(StderrCount::default());
    let bytes = vec![0xa5; STDERR_RESERVE * 7 + 1];
    drain_stderr(bytes.as_slice(), counts.clone()).await;
    assert_eq!(counts.read.load(Ordering::Relaxed), bytes.len() as u64);
}

#[tokio::test]
async fn bounded_dispatch_refusal_stops_before_the_next_frame() {
    let first = frame::encode(&Message::Ready { generation: 7 }).unwrap();
    let second = frame::encode(&Message::Ready { generation: 8 }).unwrap();
    let bytes = [first, second.clone()].concat();
    let mut input = bytes.as_slice();
    let seen = Arc::new(AtomicU64::new(0));
    let count = seen.clone();
    let dispatch: Dispatch = Arc::new(move |received| {
        assert_eq!(received.message, Message::Ready { generation: 7 });
        count.fetch_add(1, Ordering::Relaxed);
        Err(WireError::Protocol)
    });
    assert!(matches!(read_frames(&mut input, dispatch).await, Err(WireError::Protocol)));
    assert_eq!(seen.load(Ordering::Relaxed), 1);
    assert_eq!(input, second);
}
