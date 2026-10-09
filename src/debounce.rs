//! The per-key rule: a press that follows the same key's last forwarded
//! release by no more than the threshold is chatter.
//!
//! Dropped presses don't move the reference point, so a burst of bounces is
//! measured from the first release and can't keep a key suppressed forever.

use crate::sys::KEY_CNT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Forward,
    /// A press this many nanoseconds after the key's last release.
    Chatter(u64),
    /// A release or repeat for a key whose press was never forwarded.
    NotPressed,
}

pub struct Debounce {
    threshold_ns: u64,
    /// 0 means the key was never released.
    last_up_ns: [u64; KEY_CNT],
    pressed: [u64; KEY_CNT / 64],
}

impl Debounce {
    pub fn new(threshold_ns: u64) -> Self {
        Self {
            threshold_ns,
            last_up_ns: [0; KEY_CNT],
            pressed: [0; KEY_CNT / 64],
        }
    }

    pub fn is_pressed(&self, code: u16) -> bool {
        let i = code as usize;
        i < KEY_CNT && (self.pressed[i / 64] >> (i % 64)) & 1 == 1
    }

    /// Overrides the pressed state, keeping the last release time.
    pub fn reset(&mut self, code: u16, pressed: bool) {
        if (code as usize) < KEY_CNT {
            self.set_pressed(code.into(), pressed);
        }
    }

    fn set_pressed(&mut self, i: usize, on: bool) {
        if on {
            self.pressed[i / 64] |= 1 << (i % 64);
        } else {
            self.pressed[i / 64] &= !(1 << (i % 64));
        }
    }

    /// Judges an `EV_KEY` event: `value` is 0 for release, 1 for press,
    /// 2 for autorepeat; `t_ns` is its timestamp.
    pub fn key(&mut self, code: u16, value: i32, t_ns: u64) -> Verdict {
        let i = code as usize;
        if i >= KEY_CNT {
            return Verdict::Forward;
        }
        let pressed = self.is_pressed(code);
        match value {
            0 if pressed => {
                self.set_pressed(i, false);
                self.last_up_ns[i] = t_ns;
                Verdict::Forward
            }
            1 if !pressed => {
                let last = self.last_up_ns[i];
                let gap = t_ns.saturating_sub(last);
                if last != 0 && gap <= self.threshold_ns {
                    return Verdict::Chatter(gap);
                }
                self.set_pressed(i, true);
                Verdict::Forward
            }
            1 => Verdict::Forward,
            _ if pressed => Verdict::Forward,
            _ => Verdict::NotPressed,
        }
    }
}
