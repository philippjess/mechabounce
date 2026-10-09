use mechabounce::debounce::{Debounce, Mode, Verdict};

use crate::MS;

const A: u16 = 30;
const B: u16 = 48;
const T0: u64 = 1_000 * MS;

#[test]
fn first_press_and_release_pass() {
    let mut d = Debounce::new(30 * MS, Mode::DropPress);
    assert_eq!(d.key(A, 1, T0), Verdict::Forward);
    assert!(d.is_pressed(A));
    assert_eq!(d.key(A, 0, T0 + 5 * MS), Verdict::Forward);
    assert!(!d.is_pressed(A));
}

#[test]
fn press_right_after_release_is_chatter() {
    let mut d = Debounce::new(30 * MS, Mode::DropPress);
    d.key(A, 1, T0);
    d.key(A, 0, T0 + 50 * MS);
    assert_eq!(d.key(A, 1, T0 + 58 * MS), Verdict::Chatter(8 * MS));
    assert!(!d.is_pressed(A));
    assert_eq!(d.key(A, 0, T0 + 60 * MS), Verdict::NotPressed);
}

#[test]
fn threshold_is_inclusive() {
    let mut d = Debounce::new(30 * MS, Mode::DropPress);
    d.key(A, 1, T0);
    d.key(A, 0, T0 + 10 * MS);
    assert_eq!(d.key(A, 1, T0 + 40 * MS), Verdict::Chatter(30 * MS));
    assert_eq!(d.key(A, 1, T0 + 40 * MS + 1), Verdict::Forward);
}

#[test]
fn bounce_burst_is_measured_from_the_first_release() {
    let mut d = Debounce::new(30 * MS, Mode::DropPress);
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
    let mut d = Debounce::new(30 * MS, Mode::DropPress);
    d.key(A, 1, T0);
    d.key(A, 0, T0 + 10 * MS);
    assert_eq!(d.key(B, 1, T0 + 11 * MS), Verdict::Forward);
    assert!(matches!(d.key(A, 1, T0 + 12 * MS), Verdict::Chatter(_)));
}

#[test]
fn repeats_follow_the_pressed_state() {
    let mut d = Debounce::new(30 * MS, Mode::DropPress);
    assert_eq!(d.key(A, 2, T0), Verdict::NotPressed);
    d.key(A, 1, T0);
    assert_eq!(d.key(A, 2, T0 + 500 * MS), Verdict::Forward);
}

#[test]
fn out_of_range_codes_pass() {
    let mut d = Debounce::new(30 * MS, Mode::DropPress);
    assert_eq!(d.key(0x300, 0, T0), Verdict::Forward);
    assert!(!d.is_pressed(0x300));
}

fn defer() -> Debounce {
    Debounce::new(30 * MS, Mode::DeferRelease)
}

fn due(d: &mut Debounce, now: u64) -> Vec<u16> {
    let mut codes = Vec::new();
    d.take_due(now, |c| codes.push(c));
    codes
}

#[test]
fn deferred_key_stays_held_through_a_bounce_while_pressing() {
    let mut d = defer();
    assert_eq!(d.key(A, 1, T0), Verdict::Forward);
    assert_eq!(d.key(A, 0, T0 + 3 * MS), Verdict::Deferred);
    assert_eq!(due(&mut d, T0 + 6 * MS), []);
    assert_eq!(d.key(A, 1, T0 + 6 * MS), Verdict::Chatter(3 * MS));
    assert!(d.is_pressed(A));
    assert_eq!(d.next_deadline(), None);
    assert_eq!(d.key(A, 0, T0 + 100 * MS), Verdict::Deferred);
    assert!(d.is_pressed(A));
    assert_eq!(due(&mut d, T0 + 130 * MS), []);
    assert_eq!(due(&mut d, T0 + 130 * MS + 1), [A]);
    assert!(!d.is_pressed(A));
}

#[test]
fn deferred_chatter_on_release_yields_one_release() {
    let mut d = defer();
    d.key(A, 1, T0);
    for t in [50, 54, 58] {
        assert_eq!(d.key(A, 0, T0 + t * MS), Verdict::Deferred, "up at +{t} ms");
        assert_eq!(
            d.key(A, 1, T0 + (t + 2) * MS),
            Verdict::Chatter(2 * MS),
            "down at +{} ms",
            t + 2
        );
    }
    assert_eq!(d.key(A, 0, T0 + 62 * MS), Verdict::Deferred);
    assert_eq!(d.next_deadline(), Some(T0 + 92 * MS));
    assert_eq!(due(&mut d, T0 + 200 * MS), [A]);
    assert_eq!(due(&mut d, T0 + 300 * MS), []);
}

#[test]
fn deferred_double_tap_beyond_the_threshold_passes() {
    let mut d = defer();
    d.key(A, 1, T0);
    d.key(A, 0, T0 + 50 * MS);
    assert_eq!(due(&mut d, T0 + 81 * MS), [A]);
    assert_eq!(d.key(A, 1, T0 + 81 * MS), Verdict::Forward);
    assert!(d.is_pressed(A));
}

#[test]
fn deferred_threshold_is_inclusive() {
    let mut d = defer();
    d.key(A, 1, T0);
    d.key(A, 0, T0 + 10 * MS);
    assert_eq!(d.key(A, 1, T0 + 40 * MS), Verdict::Chatter(30 * MS));

    d.key(B, 1, T0);
    d.key(B, 0, T0 + 10 * MS);
    assert_eq!(due(&mut d, T0 + 40 * MS), []);
    assert_eq!(due(&mut d, T0 + 40 * MS + 1), [B]);
    assert_eq!(d.key(B, 1, T0 + 40 * MS + 1), Verdict::Forward);
}

#[test]
fn due_releases_go_out_in_deadline_order() {
    let mut d = defer();
    d.key(A, 1, T0);
    d.key(B, 1, T0);
    assert_eq!(d.next_deadline(), None);
    d.key(B, 0, T0 + 5 * MS);
    d.key(A, 0, T0 + 10 * MS);
    assert_eq!(d.next_deadline(), Some(T0 + 35 * MS));
    assert_eq!(due(&mut d, T0 + 36 * MS), [B]);
    assert_eq!(d.next_deadline(), Some(T0 + 40 * MS));
    d.key(B, 1, T0 + 36 * MS);
    d.key(B, 0, T0 + 37 * MS);
    assert_eq!(due(&mut d, T0 + 100 * MS), [A, B]);
    assert_eq!(d.next_deadline(), None);
}

#[test]
fn reset_and_release_now_drop_a_pending_release() {
    let mut d = defer();
    d.key(A, 1, T0);
    d.key(A, 0, T0 + 5 * MS);
    d.reset(A, true);
    assert!(d.is_pressed(A));
    assert_eq!(d.next_deadline(), None);
    d.key(A, 0, T0 + 6 * MS);
    d.release_now(A, T0 + 7 * MS);
    assert!(!d.is_pressed(A));
    assert_eq!(d.next_deadline(), None);
    // No reference point left, so a press passes at once.
    assert_eq!(d.key(A, 1, T0 + 8 * MS), Verdict::Forward);
}

#[test]
fn repeats_and_releases_while_a_release_is_pending_are_dropped() {
    let mut d = defer();
    assert_eq!(d.key(A, 0, T0), Verdict::NotPressed);
    d.key(A, 1, T0);
    assert_eq!(d.key(A, 2, T0 + 500 * MS), Verdict::Forward);
    assert_eq!(d.key(A, 0, T0 + 600 * MS), Verdict::Deferred);
    assert_eq!(d.key(A, 2, T0 + 610 * MS), Verdict::NotPressed);
    assert_eq!(d.key(A, 0, T0 + 615 * MS), Verdict::NotPressed);
    assert_eq!(d.next_deadline(), Some(T0 + 630 * MS));
}

#[test]
fn drop_press_never_defers() {
    let mut d = Debounce::new(30 * MS, Mode::DropPress);
    d.key(A, 1, T0);
    d.key(A, 0, T0 + 5 * MS);
    assert_eq!(d.next_deadline(), None);
    assert_eq!(due(&mut d, T0 + 100 * MS), []);
}
