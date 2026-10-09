//! Frame-level filtering of the source's event stream.
//!
//! The kernel groups events into frames ended by `SYN_REPORT`. Frames are
//! kept intact: a frame loses only the dropped keys (and their `MSC_SCAN`),
//! and a frame left with nothing to say is dropped whole.

use crate::debounce::{Debounce, Verdict};
use crate::sys::{
    EV_KEY, EV_LED, EV_MSC, EV_REP, EV_SND, EV_SYN, InputEvent, KEY_CNT, KeyBits, MSC_SCAN,
    SYN_DROPPED, SYN_REPORT, set_bit, test_bit,
};

pub struct Filter {
    keys: Debounce,
    /// The virtual device autorepeats by itself, so the source's repeats go.
    soft_repeat: bool,
    /// Events stamped before the grab already reached everyone else.
    ignore_before_ns: u64,
    verbosity: u8,
    out: Vec<InputEvent>,
    /// `out[..frame_start]` holds complete frames, the rest the open one.
    frame_start: usize,
    frame_has_payload: bool,
    /// An `MSC_SCAN` waiting for the key event it belongs to.
    pending_scan: Option<InputEvent>,
    /// After `SYN_DROPPED`, until the next `SYN_REPORT`.
    dropping: bool,
    /// Keys held down on the virtual device, as of the last complete frame.
    emitted: KeyBits,
}

impl Filter {
    pub fn new(threshold_ns: u64, soft_repeat: bool, ignore_before_ns: u64, verbosity: u8) -> Self {
        Self {
            keys: Debounce::new(threshold_ns),
            soft_repeat,
            ignore_before_ns,
            verbosity,
            out: Vec::with_capacity(256),
            frame_start: 0,
            frame_has_payload: false,
            pending_scan: None,
            dropping: false,
            emitted: Default::default(),
        }
    }

    /// Feeds one source event. Returns true when the kernel dropped events
    /// and [`Self::resync`] must be called before feeding more.
    pub fn push(&mut self, ev: InputEvent) -> bool {
        if ev.time_ns() < self.ignore_before_ns {
            return false;
        }
        match (ev.type_, ev.code) {
            (EV_SYN, SYN_REPORT) => {
                if self.dropping {
                    self.dropping = false;
                    return true;
                }
                self.out.extend(self.pending_scan.take());
                if self.frame_has_payload {
                    for e in &self.out[self.frame_start..] {
                        if e.type_ == EV_KEY && matches!(e.value, 0 | 1) {
                            set_bit(&mut self.emitted, e.code.into(), e.value == 1);
                        }
                    }
                    self.out.push(ev);
                    self.frame_start = self.out.len();
                } else {
                    self.out.truncate(self.frame_start);
                }
                self.frame_has_payload = false;
            }
            (EV_SYN, SYN_DROPPED) => {
                self.out.truncate(self.frame_start);
                self.pending_scan = None;
                self.frame_has_payload = false;
                self.dropping = true;
            }
            _ if self.dropping => {}
            (EV_MSC, MSC_SCAN) => {
                if let Some(prev) = self.pending_scan.replace(ev) {
                    self.out.push(prev);
                }
            }
            (EV_KEY, code) => {
                let scan = self.pending_scan.take();
                if self.forward_key(code, ev.value, ev.time_ns()) {
                    self.out.extend(scan);
                    self.out.push(ev);
                    self.frame_has_payload = true;
                }
            }
            // Echoes of the LED, sound and repeat settings we write
            // ourselves. Sound echoes pass even unchanged, so relaying them
            // would loop forever.
            (EV_LED | EV_SND | EV_REP, _) => {}
            // Passengers: they only go out inside a frame that carries more.
            (EV_MSC | EV_SYN, _) => self.out.push(ev),
            _ => {
                self.out.push(ev);
                self.frame_has_payload = true;
            }
        }
        false
    }

    fn forward_key(&mut self, code: u16, value: i32, t_ns: u64) -> bool {
        if value == 2 && self.soft_repeat {
            return false;
        }
        match self.keys.key(code, value, t_ns) {
            Verdict::Forward => {
                if self.verbosity >= 2 {
                    eprintln!(
                        "key {code} {}",
                        ["up", "down", "repeat"].get(value as usize).unwrap_or(&"?")
                    );
                }
                true
            }
            Verdict::Chatter(gap_ns) => {
                if self.verbosity >= 1 {
                    eprintln!(
                        "filtered key {code} down: {:.1} ms after its release",
                        gap_ns as f64 / 1e6
                    );
                }
                false
            }
            Verdict::NotPressed => {
                if self.verbosity >= 1 && value == 0 {
                    eprintln!("filtered key {code} up: its press was filtered");
                }
                false
            }
        }
    }

    /// After a `SYN_DROPPED`: releases every key held down on the virtual
    /// device that `held`, the real device's key state, says is up. Keys
    /// held but not down on the virtual device stay up; the user presses
    /// them again.
    pub fn resync(&mut self, held: &KeyBits, t_ns: u64) {
        let mut released = false;
        for code in 0..KEY_CNT as u16 {
            let down = test_bit(&self.emitted, code.into());
            // The discarded events may have moved the debounce state.
            self.keys.reset(code, down);
            if down && !test_bit(held, code.into()) {
                self.keys.key(code, 0, t_ns);
                set_bit(&mut self.emitted, code.into(), false);
                self.out.push(InputEvent::new(EV_KEY, code, 0));
                released = true;
            }
        }
        if released {
            self.out.push(InputEvent::syn_report());
        }
        self.frame_start = self.out.len();
    }

    /// Complete frames ready for the virtual device.
    pub fn ready(&self) -> &[InputEvent] {
        &self.out[..self.frame_start]
    }

    /// Forgets the frames returned by [`Self::ready`], keeping the open one.
    pub fn consume(&mut self) {
        self.out.drain(..self.frame_start);
        self.frame_start = 0;
    }
}
