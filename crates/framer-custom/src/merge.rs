//! Seam trimming and chain assembly: echo removal between chained legs.
//!
//! A continuation leg sometimes re-emits its anchor tail verbatim. The trim
//! strips that echo (anchor-suffix vs leg-prefix match) so the assembled doc
//! holds one copy. All matching is byte-exact and char-boundary-safe.
//!
//! MEMORY: every function here borrows -- trim returns a slice, assemble
//! takes a slice of legs and builds ONE output. No per-leg clones.

/// Minimum echo length worth stripping. Below this, coincidental matches
/// (common words) would eat real content.
pub const MIN_OVERLAP: usize = 20;
/// Anchor window for assembly: wide enough to catch long verbatim restarts
/// (production anchors are ~350 chars; the old 40-char window missed any
/// echo longer than itself).
pub const ASSEMBLE_WINDOW: usize = 512;
/// Upper bound on the overlap scan: echoes longer than this cannot be anchor
/// replays (anchors are ~350 chars), so scanning further only burns CPU.
pub const MAX_SCAN: usize = 1024;

/// Longest k with suffix(anchor_tail, k) == prefix(new_text, k), scanned
/// largest-first for early exit. Returns 0 below MIN_OVERLAP.
fn overlap_len(new_text: &str, anchor_tail: &str) -> usize {
    let (nb, ab) = (new_text.as_bytes(), anchor_tail.as_bytes());
    let max = nb.len().min(ab.len()).min(MAX_SCAN);
    if max < MIN_OVERLAP {
        return 0;
    }
    for k in (MIN_OVERLAP..=max).rev() {
        // Both slice ends must sit on char boundaries (model text carries multi-byte
        // ⟦⟧ markers; slicing mid-char panics the chain -> 500 -> client retry storm).
        if !new_text.is_char_boundary(k) || !anchor_tail.is_char_boundary(ab.len() - k) {
            continue;
        }
        if ab[ab.len() - k..] == nb[..k] {
            return k;
        }
    }
    0
}

/// Strip a leading anchor echo from a fresh leg. Returns a SLICE of the
/// input (zero-copy); unchanged input when no echo of at least MIN_OVERLAP
/// chars is present.
pub fn trim_overlap<'a>(new_text: &'a str, anchor_tail: &str) -> &'a str {
    let n = overlap_len(new_text, anchor_tail);
    if n >= MIN_OVERLAP {
        &new_text[n..]
    } else {
        new_text
    }
}

/// True when a trimmed leg carries nothing worth keeping: the chain's stop
/// signal (the model has run dry or the leg raced empty).
pub fn is_near_zero(new_leg_after_trim: &str) -> bool {
    new_leg_after_trim.trim().len() < 40
}

/// Last up-to-ASSEMBLE_WINDOW chars, widened forward to a char boundary.
/// Borrowed slice, never splits multi-byte content, never panics.
fn tail_window(text: &str) -> &str {
    if text.len() <= ASSEMBLE_WINDOW {
        return text;
    }
    let mut start = text.len() - ASSEMBLE_WINDOW;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// Concatenate legs in order, trimming each leg's anchor echo against the
/// running result's tail. Safe on multi-byte content by construction.
/// Single allocation: sized from the input sum up front.
pub fn assemble(legs: &[String]) -> String {
    let mut out = String::with_capacity(legs.iter().map(|l| l.len()).sum::<usize>());
    for leg in legs {
        out.push_str(trim_overlap(leg, tail_window(&out)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlap_never_splits_multibyte_markers() {
        // ⟦ is 3 bytes in UTF-8; every k in the scan must not panic.
        let anchor = "answer text here ⟦FRZ-END⟧ plus padding words following along";
        let new = "text here ⟦FRZ-END⟧ plus padding words following along and more";
        let _ = trim_overlap(new, anchor);
        let _ = trim_overlap("⟦PAD-0001⟧ x", "y ⟦PAD-0001⟧ x");
        let _ = assemble(&["a⟦b".to_string(), "b⟧c".to_string()]);
    }

    #[test]
    fn exact_echo_trimmed() {
        let anchor = "the quick brown fox jumps over the lazy dog ship hull";
        let new = "the quick brown fox jumps over the lazy dog ship hull and sailed on";
        assert_eq!(trim_overlap(new, anchor), " and sailed on");
    }
    #[test]
    fn partial_left_intact() {
        let anchor = "abcdefghij12345";
        let new = "abcdefghijXYZ rest of long content here padding!!";
        assert_eq!(trim_overlap(new, anchor), new);
    }
    #[test]
    fn no_overlap_intact() {
        assert_eq!(trim_overlap("totally new content here", "old tail here"), "totally new content here");
    }
    #[test]
    fn assemble_trims_leg2_echo() {
        let l1 = "AAA start of the long piece about ships and more text here!".to_string();
        let echo: String = l1[l1.len() - 30..].to_string();
        let l2 = echo.clone() + " and then the story continues onward here.";
        let l3 = " Final leg with fresh words and a clean ending here.".to_string();
        let got = assemble(&[l1.clone(), l2, l3.clone()]);
        assert!(got.starts_with("AAA start"));
        assert!(got.contains("story continues"));
        assert!(got.ends_with("clean ending here."));
        assert_eq!(got.matches(&echo).count(), 1);
    }
    #[test]
    fn assemble_catches_long_echo() {
        let l1 = format!("{}{}", "setup text here. ", "E".repeat(100));
        let echo = l1[l1.len() - 100..].to_string();
        let l2 = format!("{} plus fresh continuation words here.", echo);
        let got = assemble(&[l1, l2]);
        assert_eq!(got.matches(&echo).count(), 1);
        assert!(got.contains("fresh continuation"));
    }
    #[test]
    fn assemble_multibyte_never_panics() {
        let l1 = format!("héllo wörld {} 🌍🌍🌍 end one", "x".repeat(600));
        let l2 = "second leg with émojis 🚀 and more words here.".to_string();
        let got = assemble(&[l1.clone(), l2.clone()]);
        assert!(got.starts_with("héllo"));
        assert!(got.contains("🚀"));
    }
    #[test]
    fn near_zero_flags() {
        assert!(is_near_zero(""));
        assert!(is_near_zero("   "));
        assert!(!is_near_zero(&"x".repeat(100)));
    }
}
