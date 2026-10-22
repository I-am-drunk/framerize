//! What a 403 actually means.
//!
//! Framerize posts to `/ai/v3/*`, which are OpenRouter PASSTHROUGH routes. So a
//! 403 on that path has two completely different origins that demand opposite
//! handling:
//!
//! 1. **Framer's edge / Castle** refuses the request before it is proxied. This
//!    IS account health: the device is being risk-scored. Cool the account.
//! 2. **The upstream provider** (via OpenRouter) refuses the generation —
//!    region/model policy, content filter, provider key. This is NOT account
//!    health. The account is fine; the same request would be refused from any
//!    device. Cooling the account punishes the wrong thing.
//!
//! Until 2026-10-04 every 403 was classified as (1) because the body was
//! discarded at the match site. Measured consequence over one day: 1,721 403s,
//! of which only 322 carried Castle's fast-refusal signature and 1,399 were
//! upstream refusals arriving ~2.2s in with an OpenRouter `request_id`. Those
//! 1,399 drove healthy accounts to cooldown step 71 (the 300s cap), which took
//! 2 of 3 accounts out of rotation, which concentrated all load onto the
//! remaining one, which produced more 429s — a feedback loop that looked
//! exactly like "Castle is refusing us" while Castle was minting 6,192/6,192
//! tokens successfully.
//!
//! The rule: attribute a 403 to the account only on POSITIVE evidence that
//! Framer's edge refused it. Ambiguity goes to the upstream bucket, because a
//! false "upstream" costs one failed request while a false "risk-refused"
//! costs an account for five minutes.

/// How a 403 should be handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Framer's edge / Castle refused this device. Account health.
    Castle,
    /// The upstream provider refused the generation. Not account health.
    Upstream,
}

/// Why the existing policy chose its bucket. This records a heuristic,
/// not a verdict about token validity or the party that caused a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalBasis {
    ProviderMarker,
    FramerSecurityEnvelope,
    FramerMarker,
    ElapsedFallback,
}

/// Bounded policy evidence. It never retains body text or request identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefusalObservation {
    pub policy: Refusal,
    pub basis: RefusalBasis,
}

impl Refusal {
    /// The error string the pipeline matches on. `Castle` MUST stay exactly
    /// `risk-refused`: rotation and `record_account_health` compare against it.
    pub fn as_error(self, path: &str, body: &str) -> String {
        match self {
            // Exact sentinel — do not decorate (framer_core matches on equality).
            Refusal::Castle => "risk-refused".into(),
            Refusal::Upstream => {
                let clipped = &body[..body.len().min(500)];
                format!("upstream 403 on {path}: {clipped}")
            }
        }
    }
}

/// Markers that only OpenRouter / a downstream provider emits. Their presence
/// proves the request reached the proxy, i.e. Framer's edge ADMITTED it.
const UPSTREAM_MARKERS: &[&str] = &[
    "\"request_id\"",   // OpenRouter stamps every generation
    "gen-",             // ...and its ids are gen-<epoch>-<rand>
    "provider",
    "moderation",
    "No endpoints found",
    "No allowed providers",
    "error_type",       // OpenRouter's error envelope, not Framer's
    "flagged",
];

/// Markers of Framer's own edge refusal. Short, generic bodies.
const CASTLE_MARKERS: &[&str] = &["Forbidden", "castle", "risk", "challenge", "captcha"];

/// Classify a 403 from its body.
///
/// `elapsed_ms` is a tiebreaker only, never the primary signal: Framer's edge
/// refuses in ~250ms (p50 281ms measured) because it never proxies, while an
/// upstream refusal costs a round trip through OpenRouter (p50 2,219ms). Timing
/// alone would be fragile under load, so it decides only when the body is empty
/// or says nothing either way.
pub fn classify_403(body: &str, elapsed_ms: u64) -> Refusal {
    observe_403(body, elapsed_ms).policy
}

/// Preserve exactly the legacy classification while exposing its basis.
/// Callers must not treat the elapsed fallback as positive body evidence.
pub fn observe_403(body: &str, elapsed_ms: u64) -> RefusalObservation {
    let lower = body.to_ascii_lowercase();

    // Positive upstream evidence wins outright: these strings cannot appear
    // unless the request was proxied, which means the edge let it through.
    if UPSTREAM_MARKERS
        .iter()
        .any(|m| lower.contains(&m.to_ascii_lowercase()))
    {
        return RefusalObservation { policy: Refusal::Upstream,
            basis: RefusalBasis::ProviderMarker };
    }

    // Framer's structured edge refusal can arrive after browser warm-up and
    // internal retries. That elapsed time is not evidence of a provider error.
    // Require the complete envelope; an isolated reason string is ambiguous.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        let error = &value["error"];
        if error["status"] == 403
            && error["message"] == "This request was blocked by our security checks"
            && error["data"]["reason"] == "request_blocked"
            && error["ref"].as_str().is_some_and(|s| !s.is_empty())
            && error["timestamp"].as_str().is_some_and(|s| !s.is_empty())
        {
            return RefusalObservation { policy: Refusal::Castle,
                basis: RefusalBasis::FramerSecurityEnvelope };
        }
    }

    if CASTLE_MARKERS
        .iter()
        .any(|m| lower.contains(&m.to_ascii_lowercase()))
    {
        return RefusalObservation { policy: Refusal::Castle,
            basis: RefusalBasis::FramerMarker };
    }

    // No signal in the body. A refusal that took longer than any measured edge
    // refusal (989ms was the slowest of 322) was almost certainly proxied.
    let policy = if elapsed_ms >= 1_000 {
        Refusal::Upstream
    } else {
        Refusal::Castle
    };
    RefusalObservation { policy, basis: RefusalBasis::ElapsedFallback }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAMER_SECURITY_REFUSAL: &str =
        include_str!("../../../reference/fixtures/framer-security-refusal.json");

    #[test]
    fn structured_framer_refusal_is_castle_even_after_browser_retries() {
        for elapsed_ms in [250, 2_219, 14_280, 60_000] {
            let refusal = classify_403(FRAMER_SECURITY_REFUSAL, elapsed_ms);
            assert_eq!(refusal, Refusal::Castle);
            assert_eq!(refusal.as_error("/ai/v3/responses/", FRAMER_SECURITY_REFUSAL), "risk-refused");
        }
    }

    #[test]
    fn provider_evidence_still_overrides_a_structured_refusal() {
        let mut body: serde_json::Value = serde_json::from_str(FRAMER_SECURITY_REFUSAL).unwrap();
        body["request_id"] = "gen-fixture".into();
        assert_eq!(classify_403(&body.to_string(), 14_280), Refusal::Upstream);
    }

    #[test]
    fn a_reason_without_the_framer_envelope_is_ambiguous() {
        let body = r#"{"error":{"data":{"reason":"request_blocked"}}}"#;
        assert_eq!(classify_403(body, 14_280), Refusal::Upstream);
    }

    #[test]
    fn openrouter_refusals_are_not_account_health() {
        // The exact body that drove 1,399 false cooldowns on 2026-10-04.
        let body = r#"{"type":"error","error":{"type":"not_found_error","message":"No endpoints found for anthropic/claude-opus-4.","error_type":"not_found"},"request_id":"gen-1791120327-Lszyr2Z1D8g"}"#;
        assert_eq!(classify_403(body, 2_219), Refusal::Upstream);
    }

    #[test]
    fn framer_edge_forbidden_is_castle() {
        assert_eq!(
            classify_403(r#"{"error":{"message":"Forbidden"}}"#, 281),
            Refusal::Castle
        );
    }

    #[test]
    fn a_request_id_outweighs_a_fast_response() {
        // Body evidence is primary; timing must not override it. An upstream
        // refusal that happens to be fast is still not the account's fault.
        let body = r#"{"error":{"message":"Forbidden by provider"},"request_id":"gen-1"}"#;
        assert_eq!(classify_403(body, 120), Refusal::Upstream);
    }

    #[test]
    fn empty_body_falls_back_to_timing() {
        // Slower than every measured edge refusal => it was proxied.
        assert_eq!(classify_403("", 2_200), Refusal::Upstream);
        // Edge-refusal latency with nothing to go on => treat as Castle, which
        // is the conservative choice for an UNPROXIED-looking refusal.
        assert_eq!(classify_403("", 250), Refusal::Castle);
    }

    #[test]
    fn castle_sentinel_is_exact_so_rotation_still_matches() {
        // framer_core compares `e == "risk-refused"`; decorating it would
        // silently disable cooldown entirely.
        assert_eq!(Refusal::Castle.as_error("/ai/v3/messages/", "x"), "risk-refused");
    }

    #[test]
    fn upstream_error_keeps_the_body_for_debugging() {
        let e = Refusal::Upstream.as_error("/ai/v3/messages/", "No endpoints found");
        assert!(e.contains("No endpoints found"), "{e}");
        assert!(e.contains("/ai/v3/messages/"), "{e}");
        assert_ne!(e, "risk-refused");
    }

    #[test]
    fn a_long_body_is_clipped() {
        let big = "x".repeat(5_000);
        assert!(Refusal::Upstream.as_error("/p", &big).len() < 600);
    }
}
