use mechabounce::debounce::{Debounce, Verdict};

use crate::MS;

const A: u16 = 30;
const B: u16 = 48;
const T0: u64 = 1_000 * MS;

#[test]
fn first_press_and_release_pass() {
    let mut d = Debounce::new(30 * MS);
    assert_eq!(d.key(A, 1, T0), Verdict::Forward);
    assert!(d.is_pressed(A));
    assert_eq!(d.key(A, 0, T0 + 5 * MS), Verdict::Forward);
    assert!(!d.is_pressed(A));
}

#[test]
fn press_right_after_release_is_chatter() {
    let mut d = Debounce::new(30 * MS);
    d.key(A, 1, T0);
    d.key(A, 0, T0 + 50 * MS);
    assert_eq!(d.key(A, 1, T0 + 58 * MS), Verdict::Chatter(8 * MS));
    assert!(!d.is_pressed(A));
    assert_eq!(d.key(A, 0, T0 + 60 * MS), Verdict::NotPressed);
}

#[test]
fn threshold_is_inclusive() {
    let mut d = Debounce::new(30 * MS);
    d.key(A, 1, T0);
    d.key(A, 0, T0 + 10 * MS);
    assert_eq!(d.key(A, 1, T0 + 40 * MS), Verdict::Chatter(30 * MS));
    assert_eq!(d.key(A, 1, T0 + 40 * MS + 1), Verdict::Forward);
}

#[test]
fn bounce_burst_is_measured_from_the_first_release() {
    let mut d = Debounce::new(30 * MS);
    d.key(A, 1, T0);
    d.key(A, 0, T0 + 10 * MS);
    for t in [12, 18, 24, 39] {
        assert!(
            matches!(d.key(A, 1, T0 + t * MS), Verdict::Chatter(_)),
            "press at +{t} ms"
        );
        assert_eq!(d.key(A, 0, T0 + (t + 1) * MS), Verdict::NotPressed);
    }
    // 31 ms after the release that was forwarded, not after the last bounce.
    assert_eq!(d.key(A, 1, T0 + 41 * MS), Verdict::Forward);
}

#[test]
fn keys_are_independent() {
    let mut d = Debounce::new(30 * MS);
    d.key(A, 1, T0);
    d.key(A, 0, T0 + 10 * MS);
    assert_eq!(d.key(B, 1, T0 + 11 * MS), Verdict::Forward);
    assert!(matches!(d.key(A, 1, T0 + 12 * MS), Verdict::Chatter(_)));
}

#[test]
fn repeats_follow_the_pressed_state() {
    let mut d = Debounce::new(30 * MS);
    assert_eq!(d.key(A, 2, T0), Verdict::NotPressed);
    d.key(A, 1, T0);
    assert_eq!(d.key(A, 2, T0 + 500 * MS), Verdict::Forward);
}

#[test]
fn out_of_range_codes_pass() {
    let mut d = Debounce::new(30 * MS);
    assert_eq!(d.key(0x300, 0, T0), Verdict::Forward);
    assert!(!d.is_pressed(0x300));
}
