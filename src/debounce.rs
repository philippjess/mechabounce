//! The per-key rule: a press that follows the same key's release by no more
//! than the threshold is chatter. Two ways to act on that, see [`Mode`].
//!
//! In drop-press mode, dropped presses don't move the reference point, so a
//! burst of bounces is measured from the first release and can't keep a key
//! suppressed forever.

use crate::sys::KEY_CNT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Holds a release back for the threshold; a press within it cancels
    /// both, so the key stays held through a bounce.
    DeferRelease,
    /// Forwards releases at once and drops a press that follows one too
    /// closely. A bounce while pressing can leave a held key released.
    DropPress,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Forward,
    /// A press this many nanoseconds after the key's last release.
    Chatter(u64),
    /// A release or repeat for a key whose press was never forwarded.
    NotPressed,
    /// A release held back until [`Debounce::take_due`] hands it out.
    Deferred,
}

pub struct Debounce {
    threshold_ns: u64,
    mode: Mode,
    /// Drop-press: the key's last forwarded release, 0 meaning never.
    /// Defer-release: the key's pending release, 0 meaning none.
    last_up_ns: [u64; KEY_CNT],
    pressed: [u64; KEY_CNT / 64],
}

impl Debounce {
    pub fn new(threshold_ns: u64, mode: Mode) -> Self {
        Self {
            threshold_ns,
            mode,
            last_up_ns: [0; KEY_CNT],
            pressed: [0; KEY_CNT / 64],
        }
    }

    pub fn is_pressed(&self, code: u16) -> bool {
        let i = code as usize;
        i < KEY_CNT && (self.pressed[i / 64] >> (i % 64)) & 1 == 1
    }

    /// Overrides the pressed state and drops a pending release. Keeps the
    /// last release time in drop-press mode.
    pub fn reset(&mut self, code: u16, pressed: bool) {
        let i = code as usize;
        if i < KEY_CNT {
            self.set_pressed(i, pressed);
            if self.mode == Mode::DeferRelease {
                self.last_up_ns[i] = 0;
            }
        }
    }

    /// Marks the key released at `t_ns`, bypassing the rule.
    pub fn release_now(&mut self, code: u16, t_ns: u64) {
        let i = code as usize;
        if i < KEY_CNT {
            self.set_pressed(i, false);
            self.last_up_ns[i] = match self.mode {
                Mode::DeferRelease => 0,
                Mode::DropPress => t_ns,
            };
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
    /// 2 for autorepeat; `t_ns` is its timestamp. In defer-release mode,
    /// call [`Self::take_due`] with `t_ns` first.
    pub fn key(&mut self, code: u16, value: i32, t_ns: u64) -> Verdict {
        let i = code as usize;
        if i >= KEY_CNT {
            return Verdict::Forward;
        }
        match self.mode {
            Mode::DeferRelease => self.defer_release(i, value, t_ns),
            Mode::DropPress => self.drop_press(i, value, t_ns),
        }
    }

    fn drop_press(&mut self, i: usize, value: i32, t_ns: u64) -> Verdict {
        let pressed = self.is_pressed(i as u16);
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

    fn defer_release(&mut self, i: usize, value: i32, t_ns: u64) -> Verdict {
        let pressed = self.is_pressed(i as u16);
        let pending = self.last_up_ns[i];
        match value {
            0 if pressed && pending == 0 => {
                self.last_up_ns[i] = t_ns;
                Verdict::Deferred
            }
            0 => Verdict::NotPressed,
            1 if pending != 0 => {
                self.last_up_ns[i] = 0;
                let gap = t_ns.saturating_sub(pending);
                if gap <= self.threshold_ns {
                    Verdict::Chatter(gap)
                } else {
                    // Overdue: `take_due` wasn't called. The key stays held.
                    Verdict::Forward
                }
            }
            1 => {
                self.set_pressed(i, true);
                Verdict::Forward
            }
            _ if pressed && pending == 0 => Verdict::Forward,
            _ => Verdict::NotPressed,
        }
    }

    /// When the earliest pending release falls due: it goes out once the
    /// time is past this. Always `None` in drop-press mode.
    pub fn next_deadline(&self) -> Option<u64> {
        self.earliest().map(|(_, deadline)| deadline)
    }

    /// Releases every key whose pending release is due at `now_ns`, in the
    /// order they fell due, and calls `f` with each code.
    pub fn take_due(&mut self, now_ns: u64, mut f: impl FnMut(u16)) {
        while let Some((i, deadline)) = self.earliest() {
            if now_ns <= deadline {
                break;
            }
            self.set_pressed(i, false);
            self.last_up_ns[i] = 0;
            f(i as u16);
        }
    }

    /// The pending release with the earliest deadline, as (code, deadline).
    fn earliest(&self) -> Option<(usize, u64)> {
        if self.mode != Mode::DeferRelease {
            return None;
        }
        let mut best: Option<(usize, u64)> = None;
        for (w, &word) in self.pressed.iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let pending = self.last_up_ns[i];
                let deadline = pending + self.threshold_ns;
                if pending != 0 && best.is_none_or(|(_, d)| deadline < d) {
                    best = Some((i, deadline));
                }
            }
        }
        best
    }
}
