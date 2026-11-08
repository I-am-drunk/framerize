//! Abort-depth stops and prompt templates for the custom-route engine.
//!
//! The proven shape: every leg streams against a generous stop depth and the
//! abort lands mid-stream. When a reply ends before the stop (a race), the
//! engine retries at a halved stop rather than accepting a billed leg.

/// Default abort depth in stream events. Proven: aborts at 200-218 events
/// land mid-stream on length-guaranteed replies; every leg stays billable-0.
pub const DEFAULT_STOP: u32 = 200;
/// Floor for any stop: below this the abort lands inside model warm-up text
/// and legs come back near-empty.
pub const MIN_STOP: u32 = 40;
/// Hard ceiling: a stop above this is caller error, not intent. Bounds the
/// worst-case capture (10k deltas fit under the rung capture cap).
pub const MAX_STOP: u32 = 10_000;

/// Normalize a caller stop into [MIN_STOP, MAX_STOP].
pub fn clamped_stop(stop: u32) -> u32 {
    stop.max(MIN_STOP).min(MAX_STOP)
}

/// Retry depth after a leg raced (completed before the stop): halve, floored.
/// Proven: a short reply beating stop=200 recovered free at stop=100.
pub fn halt_after_race(prev_stop: u32) -> u32 {
    (prev_stop / 2).max(MIN_STOP)
}

/// Length-guarantee template: forces the reply past any sane stop so the
/// abort always lands mid-stream. Instruction-first by construction.
pub fn length_guarantee_suffix(min_words: usize, sections: usize) -> String {
    format!(
        "Write an exhaustive long-form explainer of at least {} words, organized as exactly {} numbered sections of at least 120 words each. Do not stop early, do not summarize, do not conclude until every section is complete.",
        min_words, sections
    )
}

/// Continuation template: the tail anchor follows this verbatim.
pub fn continue_suffix() -> String {
    "Continue EXACTLY from the end of this text. Output only strictly new content, at length, in flowing prose. Do not stop early, do not summarize, do not conclude. Do not repeat, recap, or restate anything it. Stay on the same topic in the same style:".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn floors_at_min() {
        assert_eq!(clamped_stop(0), MIN_STOP);
        assert_eq!(clamped_stop(10), MIN_STOP);
        assert_eq!(clamped_stop(200), 200);
    }
    #[test]
    fn ceilings_at_max() {
        assert_eq!(clamped_stop(u32::MAX), MAX_STOP);
        assert_eq!(clamped_stop(MAX_STOP + 1), MAX_STOP);
    }
    #[test]
    fn race_halves() {
        assert_eq!(halt_after_race(200), 100);
        assert_eq!(halt_after_race(30), MIN_STOP);
        assert_eq!(halt_after_race(40), MIN_STOP);
        assert_eq!(halt_after_race(u32::MAX), u32::MAX / 2);
    }
    #[test]
    fn suffix_contents() {
        let s = length_guarantee_suffix(1800, 14);
        assert!(s.contains("1800") && s.contains("14") && s.contains("Do not stop early"));
        assert!(continue_suffix().contains("only strictly new content"));
    }
}
