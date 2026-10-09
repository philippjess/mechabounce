use mechabounce::filter::Filter;
use mechabounce::sys::{
    EV_KEY, EV_LED, EV_MSC, EV_REL, EV_SND, EV_SYN, InputEvent, KeyBits, LED_CAPSL, MSC_SCAN,
    SYN_DROPPED, SYN_REPORT,
};

use crate::{MS, ev};

const A: u16 = 30;
const B: u16 = 48;

fn filter() -> Filter {
    Filter::new(30 * MS, true, 0, 0)
}

fn key(t: u64, code: u16, value: i32) -> [InputEvent; 3] {
    [
        ev(t, EV_MSC, MSC_SCAN, 0x70000 + code as i32),
        ev(t, EV_KEY, code, value),
        ev(t, EV_SYN, SYN_REPORT, 0),
    ]
}

fn feed(f: &mut Filter, evs: &[InputEvent]) -> Vec<InputEvent> {
    for &e in evs {
        assert!(!f.push(e));
    }
    let out = f.ready().to_vec();
    f.consume();
    out
}

#[test]
fn frames_pass_through_intact() {
    let mut f = filter();
    let press = key(0, A, 1);
    assert_eq!(feed(&mut f, &press), press);
}

#[test]
fn a_frame_with_only_chatter_vanishes() {
    let mut f = filter();
    feed(&mut f, &key(0, A, 1));
    feed(&mut f, &key(10, A, 0));
    assert_eq!(feed(&mut f, &key(15, A, 1)), []);
    assert_eq!(feed(&mut f, &key(17, A, 0)), []);
    assert_eq!(feed(&mut f, &key(50, A, 1)), key(50, A, 1));
}

#[test]
fn a_mixed_frame_loses_only_the_chatter_and_its_scan_code() {
    let mut f = filter();
    feed(&mut f, &key(0, A, 1));
    feed(&mut f, &key(10, A, 0));
    let frame = [
        ev(15, EV_MSC, MSC_SCAN, 1),
        ev(15, EV_KEY, A, 1),
        ev(15, EV_MSC, MSC_SCAN, 2),
        ev(15, EV_KEY, B, 1),
        ev(15, EV_SYN, SYN_REPORT, 0),
    ];
    assert_eq!(feed(&mut f, &frame), frame[2..]);
}

#[test]
fn source_repeats_are_dropped_when_the_kernel_repeats_for_us() {
    let mut f = filter();
    feed(&mut f, &key(0, A, 1));
    assert_eq!(
        feed(
            &mut f,
            &[ev(600, EV_KEY, A, 2), ev(600, EV_SYN, SYN_REPORT, 0)]
        ),
        []
    );
}

#[test]
fn source_repeats_pass_without_soft_repeat() {
    let mut f = Filter::new(30 * MS, false, 0, 0);
    let repeat = [ev(600, EV_KEY, A, 2), ev(600, EV_SYN, SYN_REPORT, 0)];
    assert_eq!(feed(&mut f, &repeat), []);
    feed(&mut f, &key(700, A, 1));
    assert_eq!(feed(&mut f, &repeat), repeat);
}

#[test]
fn led_echoes_are_dropped() {
    let mut f = filter();
    assert_eq!(
        feed(
            &mut f,
            &[ev(0, EV_LED, LED_CAPSL, 1), ev(0, EV_SYN, SYN_REPORT, 0)]
        ),
        []
    );
}

#[test]
fn sound_echoes_are_dropped() {
    let mut f = filter();
    assert_eq!(
        feed(&mut f, &[ev(0, EV_SND, 1, 1), ev(0, EV_SYN, SYN_REPORT, 0)]),
        []
    );
}

#[test]
fn other_events_pass() {
    let mut f = filter();
    let wheel = [ev(0, EV_REL, 8, -1), ev(0, EV_SYN, SYN_REPORT, 0)];
    assert_eq!(feed(&mut f, &wheel), wheel);
}

#[test]
fn events_from_before_the_grab_are_ignored() {
    let mut f = Filter::new(30 * MS, true, ev(100, 0, 0, 0).time_ns(), 0);
    assert_eq!(feed(&mut f, &key(99, A, 1)), []);
    assert_eq!(feed(&mut f, &key(100, B, 1)), key(100, B, 1));
}

#[test]
fn an_open_frame_waits_for_its_syn() {
    let mut f = filter();
    let [scan, press, syn] = key(0, A, 1);
    assert_eq!(feed(&mut f, &[scan, press]), []);
    assert_eq!(feed(&mut f, &[syn]), [scan, press, syn]);
}

#[test]
fn syn_dropped_discards_until_the_next_report_then_releases_stale_keys() {
    let mut f = filter();
    feed(&mut f, &key(0, A, 1));
    feed(&mut f, &key(0, B, 1));
    for e in [
        ev(5, EV_KEY, A, 0),
        ev(5, EV_SYN, SYN_DROPPED, 0),
        ev(6, EV_KEY, B, 0),
    ] {
        assert!(!f.push(e));
    }
    assert!(f.push(ev(6, EV_SYN, SYN_REPORT, 0)));
    assert_eq!(f.ready(), []);

    // The device says B is still down and A is up.
    let mut held: KeyBits = Default::default();
    held[B as usize / 64] |= 1 << (B % 64);
    f.resync(&held, ev(7, 0, 0, 0).time_ns());
    assert_eq!(
        f.ready(),
        [InputEvent::new(EV_KEY, A, 0), InputEvent::syn_report()]
    );
    f.consume();

    // A's synthetic release counts for debouncing.
    assert_eq!(feed(&mut f, &key(20, A, 1)), []);
    assert_eq!(feed(&mut f, &key(40, B, 0)), key(40, B, 0));
}
