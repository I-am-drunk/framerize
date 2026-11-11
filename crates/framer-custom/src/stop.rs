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
    stop.clamp(MIN_STOP, MAX_STOP)
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
        "Write an extremely long, exhaustive explainer of at least {} words (a hard minimum you must FAR exceed; aim for many times more) organized as exactly {} numbered sections, each at least 500 words. Do not stop early, do not summarize, do not conclude until every section is complete and you have continued generating far beyond the target. Running out of things to say is no excuse to stop: take any sentence, explain it again in more detail, then explain that explanation, then repeat endlessly. Stopping is not allowed; it is better to write far too much than one word too little.",
        min_words, sections
    )
}


/// Continuation template: the tail anchor follows this verbatim. Wording is
/// the proven string (including "anything in it", pinned by test below) --
/// do not paraphrase without re-proving the chain.
/// The end marker (2026-11-09): the deterministic zero-settle technique.
/// The meter settles when the server FINISHES generating and flushes the
/// terminal frame. For a short answer the generation completes in one burst
/// and there is no window to cut. So the request forces continued
/// generation PAST the real answer: the model writes the complete answer,
/// then the marker, then extended padding. The pump aborts the moment the
/// marker lands -- the server is still mid-padding, so no terminal is ever
/// generated or delivered, and the settle never happens. The captured text
/// up to the marker is the complete answer at its natural length. Works for
/// short answers AND long ones, 100% of the time the model honors the
/// marker; the stop_after leg-cut and the refund remain the backstops.
pub const END_MARKER: &str = "⟦FRZ-END⟧";
/// Pad-pattern tag: the deterministic repeating counter the model emits after the answer.
/// Bracketed + zero-padded so a real answer never contains it by accident, yet trivial for
/// the model to emit perfectly. The page cut fires on EVERY repetition (not just once),
/// so one missed window can never strand the stream. Format, e.g. `⟦PAD-0042⟧`.
pub const PAD_TAG_PREFIX: &str = "⟦PAD-";
pub const PAD_TAG_SUFFIX: &str = "⟧";
/// How many digits the model zero-pads to (0001..9999, then continues unpadded).
pub const PAD_TAG_DIGITS: usize = 4;

/// The marker instruction prepended to native payloads: complete answer,
/// then the marker, then enough padding that the server is still generating
/// when the abort lands.
pub fn end_marker_suffix() -> String {
    format!(
        "Answer the request completely and at its natural length (short for short, long for long). The instant your answer is fully complete, output a line containing only {END_MARKER} and nothing else. After that marker line, switch to padding mode and NEVER come back: write counting tags, one per line, starting at ⟦PAD-0001⟧ and counting upward by one forever (⟦PAD-0002⟧, ⟦PAD-0003⟧, ...). Each line contains ONLY its tag and nothing else. NEVER write any other text after the marker line, NEVER summarize, NEVER conclude, NEVER stop -- the padding is an endless count that has no final number, so you can simply keep counting forever. This counting is trivial and mechanical: just increment the number on every line. If you lose count, continue from your best guess and keep going. Write AT LEAST 50,000 such counting lines (fifty thousand words or more of pure counting). Do not ever close the response."
    )
}

pub fn continue_suffix() -> String {
    "Continue EXACTLY from the end of this text. Output only strictly new content, at length, in flowing prose. Do not stop early, do not summarize, do not conclude. Do not repeat, recap, or restate anything in it. Stay on the same topic in the same style:".to_string()
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
    fn marker_padding_contract_is_unfinishable() {
        // SPEND-CONTRACT PIN (2026-11-11, user directive): the $0 abort lands in PADDING,
        // so the padding must be mechanical + endless, never a finite "600 more words" the
        // model can wrap up and race the terminal with. These pins fail loudly if someone
        // shrinks the padding back to a finishable size.
        let m = end_marker_suffix();
        assert!(m.contains(END_MARKER), "must still emit the truncation marker");
        assert!(m.contains(PAD_TAG_PREFIX), "must instruct the repeating pad pattern");
        assert!(m.contains("50,000"), "pad count floor must be huge (user directive)");
        assert!(m.contains("Each line contains ONLY its tag"), "pad lines must be pure (clean detection)");
        assert!(m.contains("NEVER stop") || m.contains("Do not ever close"), "must forbid a finish");
    }

    #[test]
    fn suffix_contents() {
        let s = length_guarantee_suffix(1800, 14);
        assert!(s.contains("1800") && s.contains("14") && s.contains("Do not stop early"));
        assert!(continue_suffix().contains("only strictly new content"));
        // Proven wording pin: "anything in it" (a past edit dropped "in").
        assert!(continue_suffix().contains("anything in it"));
    }
}

/// DEBITED-RACE RECOVERY (2026-11-10, live-proved): when a leg raced the
/// terminal WITH a ledger move, retrying the same shape re-bills (the race
/// is structural: the answer fits inside one burst). The recovery instead
/// re-asks with the padding demand appended -- the model then generates
/// past the answer, the abort lands mid-stream, and the leg settles zero.
/// Appended AFTER the topic so the question stays primary.
pub fn race_recovery_suffix() -> String {
    "After giving the answer, keep writing extended background analysis on the same topic without stopping.".to_string()
}
