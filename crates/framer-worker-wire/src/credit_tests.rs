use super::*;

#[test]
fn incomplete_or_unsent_events_cannot_be_acknowledged() {
    let mut frontier = Frontier::default();
    assert_eq!(frontier.accepted(), 0);
    frontier.segment(0, false).unwrap();
    assert!(frontier.acknowledge(1).is_err());
    assert!(frontier.segment(2, true).is_err());
    frontier.segment(0, true).unwrap();
    frontier.acknowledge(1).unwrap();
    assert!(frontier.acknowledge(2).is_err());
    assert_eq!(frontier.accepted(), 1);
}

#[test]
fn contiguous_prefix_jumps_and_repeated_frontiers_are_legal() {
    let mut frontier = Frontier::default();
    for sequence in 0..3 { frontier.segment(sequence, true).unwrap(); }
    frontier.acknowledge(3).unwrap();
    frontier.acknowledge(3).unwrap();
    assert!(frontier.acknowledge(2).is_err());
    assert!(frontier.segment(2, true).is_err());
    assert_eq!(frontier.completed(), 3);
    assert_eq!(frontier.accepted(), 3);
}

#[test]
fn rejected_reservations_and_returns_do_not_change_credit() {
    let mut window = Window::new(capacity(128, 2, 16)).unwrap();
    let before = window.available();
    assert!(window.reserve(capacity(64, 1, 17)).is_err());
    assert_eq!(window.available(), before);
    window.reserve(capacity(64, 1, 8)).unwrap();
    let reserved = window.available();
    assert!(window.release(capacity(65, 1, 8)).is_err());
    assert_eq!(window.available(), reserved);
    window.release(capacity(64, 1, 8)).unwrap();
    assert_eq!(window.available(), before);
    assert!(window.release(capacity(1, 0, 0)).is_err());
}

#[test]
fn arithmetic_overflow_and_empty_limits_are_rejected() {
    assert!(Window::new(capacity(0, 1, 1)).is_err());
    assert!(Window::new(capacity(1, 0, 1)).is_err());
    assert!(Window::new(capacity(1, 1, 0)).is_err());
    let mut window = Window::new(capacity(u64::MAX, 1, 1)).unwrap();
    assert!(window.release(capacity(1, 0, 0)).is_err());
    assert_eq!(window.available(), capacity(u64::MAX, 1, 1));
}

fn capacity(wire_bytes: u64, slots: u64, downstream_bytes: u64) -> Capacity {
    Capacity { wire_bytes, slots, downstream_bytes }
}

#[test]
fn decoded_items_keep_slots_and_downstream_credit() {
    let mut window = Window::new(capacity(128, 2, 16)).unwrap();
    window.reserve(capacity(64, 1, 8)).unwrap();
    window.release(capacity(64, 0, 0)).unwrap();
    assert_eq!(window.outstanding(), capacity(0, 1, 8));
    window.reserve(capacity(64, 1, 8)).unwrap();
    assert!(window.reserve(capacity(1, 1, 0)).is_err());
    window.release(capacity(64, 2, 16)).unwrap();
    assert_eq!(window.outstanding(), Capacity::default());
}

#[test]
fn empty_frames_cannot_bypass_the_message_limit() {
    let mut window = Window::new(capacity(128, 1, 16)).unwrap();
    window.reserve(capacity(8, 1, 0)).unwrap();
    assert!(window.reserve(capacity(8, 1, 0)).is_err());
    assert_eq!(window.outstanding(), capacity(8, 1, 0));
}
