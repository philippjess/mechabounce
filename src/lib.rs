//! Keyboard chatter filter: grab an evdev keyboard, drop key presses that
//! follow the same key's release too closely, and re-emit everything else on
//! a uinput clone of the device.

pub mod debounce;
pub mod filter;
pub mod sys;
