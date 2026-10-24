use super::*;

const NOW: i64 = 1_800_000_000_000;

#[test]
fn resident_budget_rejects_the_previously_admissible_short_lifetime() {
    assert!(!covers_resident_deadline(NOW + 31_000, NOW, Duration::from_secs(45)));
    assert!(covers_resident_deadline(NOW + 600_000, NOW, Duration::from_secs(45)));
}

#[test]
fn short_budget_keeps_the_existing_thirty_second_minimum() {
    for lifetime in [0, 29_999, 30_000, 30_001] {
        assert!(!covers_resident_deadline(NOW + lifetime, NOW, Duration::from_secs(1)));
    }
    assert!(covers_resident_deadline(NOW + 30_002, NOW, Duration::from_secs(1)));
}

#[test]
fn expiry_must_exceed_the_full_deadline_after_clock_precision_margin() {
    let remaining = Duration::from_secs(45);
    for lifetime in [44_999, 45_000, 45_001] {
        assert!(!covers_resident_deadline(NOW + lifetime, NOW, remaining));
    }
    assert!(covers_resident_deadline(NOW + 45_002, NOW, remaining));
    assert!(!covers_resident_deadline(NOW + 45_002, NOW,
        Duration::from_millis(45_001)));
}

#[test]
fn submillisecond_remaining_budget_is_not_rounded_down() {
    let remaining = Duration::from_secs(45) + Duration::from_nanos(1);
    assert!(!covers_resident_deadline(NOW + 45_001, NOW, remaining));
    assert!(covers_resident_deadline(NOW + 45_002, NOW, remaining));
}

#[test]
fn an_elapsed_control_budget_cannot_be_revived_by_a_later_expiry() {
    assert!(!covers_resident_deadline(NOW + 600_000, NOW, Duration::ZERO));
}

#[test]
fn expired_negative_and_overflowing_lifetimes_refuse_without_panicking() {
    for (expires, now) in [(NOW - 1, NOW), (0, NOW), (i64::MIN, NOW),
        (i64::MAX, i64::MIN), (i64::MIN, i64::MAX)] {
        assert!(!covers_resident_deadline(expires, now, Duration::from_secs(45)));
    }
    assert!(!covers_resident_deadline(i64::MAX, 0, Duration::MAX));
}
