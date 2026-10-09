use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mechabounce::debounce::Mode;
use mechabounce::filter::Filter;
use mechabounce::sys::{
    EV_CNT, EV_LED, EV_REP, EV_SND, Evdev, InputEvent, KeyBits, Uinput, monotonic_ns, ones,
    test_bit,
};

const BY_ID: &str = "/dev/input/by-id";

const USAGE: &str = "\
Usage: mechabounce [OPTIONS]

Filters keyboard chatter: holds each key release back for the threshold,
so a bounce that presses the key again within it is swallowed, and
re-emits everything else on a virtual copy of the keyboard.

Options:
  -k, --keyboard DEV   Keyboard to filter: a name in /dev/input/by-id, an
                       eventN node name, or a path
                       [default: the only *-event-kbd in /dev/input/by-id]
  -t, --threshold MS   A press this soon after the key's release is
                       chatter [default: 30]
  --defer-release      Hold each release back for the threshold; a press
                       within it cancels both [default]
  --no-defer-release   Forward releases at once and drop the press after
                       a quick release instead (the original rule; a
                       bounce while pressing can lose a held key)
  -v, --verbosity N    0 quiet, 1 log filtered events, 2 log every key
                       [default: 1]
  -l, --list           List the keyboards in /dev/input/by-id
  -h, --help           Show this help
  -V, --version        Show the version
";

struct Args {
    keyboard: Option<String>,
    threshold_ms: u64,
    mode: Mode,
    verbosity: u8,
    list: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mechabounce: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let Some(args) = parse_args()? else {
        return Ok(());
    };
    if args.list {
        keyboards()?.iter().for_each(|k| println!("{k}"));
        return Ok(());
    }
    let path = resolve(args.keyboard)?;
    let ctx = |what: &'static str| {
        let path = path.display().to_string();
        move |e: io::Error| format!("{path}: {what}: {e}")
    };

    let src = Evdev::open(&path).map_err(|e| match e.kind() {
        io::ErrorKind::PermissionDenied => format!(
            "{}: {e} (run as root or in the input group)",
            path.display()
        ),
        _ => format!("{}: {e}", path.display()),
    })?;
    src.use_monotonic_clock().map_err(ctx("set clock"))?;
    // Grab first: if someone else holds the keyboard, fail before a virtual
    // one appears. Events from the meantime wait in the evdev buffer.
    let t0 = grab_when_idle(&src, args.verbosity).map_err(ctx("grab"))?;
    let ui = Uinput::clone_of(&src)
        .map_err(|e| format!("/dev/uinput: creating the virtual keyboard: {e}"))?;
    let soft_repeat = test_bit(
        &src.bits(0, EV_CNT).map_err(ctx("read capabilities"))?,
        EV_REP.into(),
    );

    if args.verbosity >= 1 {
        let name = src.name().map_err(ctx("read name"))?;
        eprintln!(
            "filtering \"{}\" ({}) with a {} ms threshold, {}",
            String::from_utf8_lossy(&name),
            path.display(),
            args.threshold_ms,
            match args.mode {
                Mode::DeferRelease => "deferring releases",
                Mode::DropPress => "dropping quick presses",
            }
        );
    }
    let mut filter = Filter::new(
        args.threshold_ms * 1_000_000,
        args.mode,
        soft_repeat,
        t0,
        args.verbosity,
    );
    relay(&src, &ui, &mut filter).map_err(ctx("relay"))
}

/// How long to wait for held keys before grabbing anyway. Some keyboard
/// interfaces report a key as permanently held.
const IDLE_WAIT_MS: u64 = 2_000;

/// Grabs once no key is held, so no release can get stuck on the other side
/// of the grab. Returns the time just before the grab.
fn grab_when_idle(src: &Evdev, verbosity: u8) -> io::Result<u64> {
    let mut held: KeyBits = Default::default();
    let mut stuck: KeyBits = Default::default();
    let mut sink = [InputEvent::default(); 64];
    let deadline = monotonic_ns() + IDLE_WAIT_MS * 1_000_000;
    let mut announced = false;
    let idle = |held: &mut KeyBits, stuck: &KeyBits| -> io::Result<bool> {
        src.key_state(held)?;
        held.iter_mut().zip(stuck).for_each(|(h, s)| *h &= !s);
        Ok(held.iter().all(|&w| w == 0))
    };
    loop {
        if idle(&mut held, &stuck)? {
            let t0 = monotonic_ns();
            src.grab(true)?;
            // A key pressed while we grabbed: let it through and retry.
            if idle(&mut held, &stuck)? {
                return Ok(t0);
            }
            src.grab(false)?;
        }
        let now = monotonic_ns();
        if now >= deadline {
            eprintln!(
                "keys {} still held after {IDLE_WAIT_MS} ms, grabbing anyway",
                codes(&held)
            );
            stuck.iter_mut().zip(&held).for_each(|(s, h)| *s |= h);
            continue;
        }
        if verbosity >= 1 && !announced {
            eprintln!("waiting for held keys to be released: {}", codes(&held));
            announced = true;
        }
        if readable(src.as_raw_fd(), (deadline - now).div_ceil(1_000_000))? {
            src.read(&mut sink)?;
        }
    }
}

fn codes(keys: &KeyBits) -> String {
    ones(keys)
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn readable(fd: i32, timeout_ms: u64) -> io::Result<bool> {
    let mut p = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let timeout = timeout_ms.min(i32::MAX as u64) as i32;
    match unsafe { libc::poll(&mut p, 1, timeout) } {
        n if n >= 0 => Ok(n > 0),
        _ => match io::Error::last_os_error() {
            e if e.kind() == io::ErrorKind::Interrupted => Ok(false),
            e => Err(e),
        },
    }
}

/// How long a deferred release waits past its deadline. It covers a press
/// that is already timestamped but not yet readable, so a bounce right at the
/// deadline can't slip through as a second keystroke.
const FLUSH_MARGIN_NS: u64 = 1_000_000;

/// Source events go through the filter to the virtual keyboard; LED and
/// sound state set on the virtual keyboard goes back to the real one.
fn relay(src: &Evdev, ui: &Uinput, filter: &mut Filter) -> io::Result<()> {
    let mut buf = [InputEvent::default(); 64];
    let mut held: KeyBits = Default::default();
    let mut fds = [src.as_raw_fd(), ui.as_raw_fd()].map(|fd| libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    });
    loop {
        let timeout = poll_timeout(filter.next_deadline());
        if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) } < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }

        if fds[0].revents != 0 {
            let n = match src.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(e) if e.raw_os_error() == Some(libc::ENODEV) => {
                    eprintln!("keyboard removed");
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            for &ev in &buf[..n] {
                if filter.push(ev) {
                    src.key_state(&mut held)?;
                    filter.resync(&held, monotonic_ns());
                }
            }
        }

        if fds[1].revents != 0 {
            let n = ui.read(&mut buf[..63])?;
            let mut k = 0;
            for i in 0..n {
                if matches!(buf[i].type_, EV_LED | EV_SND) {
                    buf[k] = buf[i];
                    k += 1;
                }
            }
            if k > 0 {
                buf[k] = InputEvent::syn_report();
                src.write(&buf[..=k])?;
            }
        }

        filter.flush_due(monotonic_ns().saturating_sub(FLUSH_MARGIN_NS));
        let ready = filter.ready();
        if !ready.is_empty() {
            ui.write(ready)?;
            filter.consume();
        }
    }
}

/// Milliseconds until a deferred release due at `deadline` should go out,
/// rounded up, or -1 to wait for events alone.
fn poll_timeout(deadline: Option<u64>) -> i32 {
    deadline.map_or(-1, |d| {
        let wait = (d + FLUSH_MARGIN_NS).saturating_sub(monotonic_ns());
        wait.div_ceil(1_000_000).min(i32::MAX as u64) as i32
    })
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut args = Args {
        keyboard: None,
        threshold_ms: 30,
        mode: Mode::DeferRelease,
        verbosity: 1,
        list: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_owned(), Some(v.to_owned())),
            _ => (arg, None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| it.next())
                .ok_or(format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "-k" | "--keyboard" => args.keyboard = Some(value()?),
            "-t" | "--threshold" => {
                args.threshold_ms = value()?
                    .parse()
                    .map_err(|_| format!("{flag} takes whole milliseconds"))?;
            }
            "-v" | "--verbosity" => {
                args.verbosity = match value()?.as_str() {
                    "0" => 0,
                    "1" => 1,
                    "2" => 2,
                    _ => return Err(format!("{flag} takes 0, 1 or 2")),
                };
            }
            "--defer-release" => args.mode = Mode::DeferRelease,
            "--no-defer-release" => args.mode = Mode::DropPress,
            "-l" | "--list" => args.list = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("mechabounce {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            _ => return Err(format!("unknown argument {flag:?}\n\n{USAGE}")),
        }
    }
    Ok(Some(args))
}

fn resolve(keyboard: Option<String>) -> Result<PathBuf, String> {
    let Some(name) = keyboard else {
        return match keyboards()?.as_slice() {
            [one] => Ok(Path::new(BY_ID).join(one)),
            [] => Err(format!("no *-event-kbd in {BY_ID}; pass --keyboard")),
            many => Err(format!(
                "several keyboards found, pick one with --keyboard:\n  {}",
                many.join("\n  ")
            )),
        };
    };
    Ok(if name.contains('/') {
        name.into()
    } else if name.starts_with("event") {
        Path::new("/dev/input").join(name)
    } else {
        Path::new(BY_ID).join(name)
    })
}

fn keyboards() -> Result<Vec<String>, String> {
    let dir = fs::read_dir(BY_ID).map_err(|e| format!("{BY_ID}: {e}"))?;
    let mut names: Vec<String> = dir
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with("-event-kbd"))
        .collect();
    names.sort();
    Ok(names)
}
