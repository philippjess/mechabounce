mod debounce;
mod e2e;
mod filter;

use mechabounce::sys::InputEvent;

const MS: u64 = 1_000_000;

/// An event stamped `t_ms` milliseconds after boot.
fn ev(t_ms: u64, type_: u16, code: u16, value: i32) -> InputEvent {
    let ns = 1_000 * MS + t_ms * MS;
    InputEvent {
        sec: (ns / 1_000_000_000) as _,
        usec: (ns % 1_000_000_000 / 1_000) as _,
        type_,
        code,
        value,
    }
}
