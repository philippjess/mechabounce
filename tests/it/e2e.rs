//! End to end through the kernel: a fake uinput keyboard as the source, the
//! built binary in the middle, and the virtual keyboard it creates as the
//! sink. Needs root for the /dev/input nodes: `just e2e`.
//!
//! The fake keyboard only has macro keys, which no keymap binds, so the
//! desktop doesn't act on what the test types.

use std::fs;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::{Duration, Instant};

use mechabounce::sys::{
    BUS_VIRTUAL, EV_KEY, EV_LED, EV_MSC, EV_SYN, Evdev, InputEvent, InputId, LED_CAPSL, MSC_SCAN,
    SYN_REPORT, Uinput, UinputBuilder,
};

const KEY_MACRO1: u16 = 0x290;
const KEY_MACRO2: u16 = 0x291;

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn fake_keyboard(name: &str) -> Uinput {
    let b = UinputBuilder::new().unwrap();
    b.enable_type(EV_KEY).unwrap();
    b.enable_code(EV_KEY, KEY_MACRO1).unwrap();
    b.enable_code(EV_KEY, KEY_MACRO2).unwrap();
    b.enable_type(EV_MSC).unwrap();
    b.enable_code(EV_MSC, MSC_SCAN).unwrap();
    b.enable_type(EV_LED).unwrap();
    b.enable_code(EV_LED, LED_CAPSL).unwrap();
    let id = InputId {
        bustype: BUS_VIRTUAL,
        vendor: 0x1209,
        product: 0x0001,
        version: 1,
    };
    b.create(id, name.as_bytes()).unwrap()
}

/// `/dev/input/eventN` of the input device `sysname` (`inputM`).
fn event_node(sysname: &str) -> Option<String> {
    let dir = fs::read_dir(Path::new("/sys/class/input").join(sysname)).ok()?;
    let event = dir
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .find(|n| n.starts_with("event"))?;
    Some(format!("/dev/input/{event}"))
}

/// The other input device named `name`: the one the binary created.
fn find_clone(name: &str, not: &str) -> Option<String> {
    fs::read_dir("/sys/class/input")
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .find_map(|sys| {
            let n = fs::read_to_string(format!("/sys/class/input/{sys}/name")).ok()?;
            (sys != not && n.trim_end() == name)
                .then(|| event_node(&sys))
                .flatten()
        })
}

fn readable(fd: i32, timeout: Duration) -> bool {
    let mut p = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut p, 1, timeout.as_millis() as _) > 0 }
}

fn key(code: u16, value: i32) -> [InputEvent; 3] {
    [
        InputEvent::new(EV_MSC, MSC_SCAN, code.into()),
        InputEvent::new(EV_KEY, code, value),
        InputEvent::syn_report(),
    ]
}

/// The sink's key frames as `(scan, code, value)` lists, ignoring frames
/// without keys (LED changes the desktop makes on the new keyboard).
fn key_frames(sink: &Evdev, want: usize) -> Vec<Vec<(u16, u16, i32)>> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut frames = Vec::new();
    let mut frame = Vec::new();
    let mut buf = [InputEvent::default(); 64];
    while frames.len() < want || readable(sink.as_raw_fd(), Duration::from_millis(100)) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || !readable(sink.as_raw_fd(), left) {
            break;
        }
        let n = sink.read(&mut buf).unwrap();
        for e in &buf[..n] {
            match (e.type_, e.code) {
                (EV_SYN, SYN_REPORT) if frame.iter().any(|&(t, _, _)| t == EV_KEY) => {
                    frames.push(std::mem::take(&mut frame))
                }
                (EV_SYN, SYN_REPORT) => frame.clear(),
                (EV_MSC | EV_KEY, _) => frame.push((e.type_, e.code, e.value)),
                _ => {}
            }
        }
    }
    frames
}

type Frame = Vec<(u16, u16, i32)>;

/// A key frame as the fake keyboard sends it.
fn frame(code: u16, value: i32) -> Frame {
    vec![(EV_MSC, MSC_SCAN, code as i32), (EV_KEY, code, value)]
}

/// A deferred release: it goes out on its own, without a scan code.
fn deferred_up(code: u16) -> Frame {
    vec![(EV_KEY, code, 0)]
}

fn ms(n: u64) {
    sleep(Duration::from_millis(n));
}

/// The binary between a fake keyboard and the virtual one it made.
struct Rig {
    sink: Evdev,
    _child: KillOnDrop,
    src: Uinput,
}

/// Runs the binary with `args` on a fake keyboard, types `keys` on it and
/// checks the key frames the virtual keyboard sends.
fn relays(tag: &str, args: &[&str], keys: impl FnOnce(&Uinput), want: &[Frame]) -> Rig {
    let name = format!("mechabounce-e2e-{}-{tag}", std::process::id());
    let src = fake_keyboard(&name);
    let src_sys = src.sysname().unwrap();
    let src_node = event_node(&src_sys).expect("no event node for the fake keyboard");

    let child = KillOnDrop(
        Command::new(env!("CARGO_BIN_EXE_mechabounce"))
            .args([
                "--keyboard",
                &src_node,
                "--threshold",
                "30",
                "--verbosity",
                "0",
            ])
            .args(args)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let sink_node = loop {
        if let Some(node) = find_clone(&name, &src_sys) {
            break node;
        }
        assert!(
            Instant::now() < deadline,
            "the binary never created its virtual keyboard"
        );
        ms(10);
    };
    ms(50);
    let sink = Evdev::open(Path::new(&sink_node)).unwrap();
    // The binary grabs right after creating the clone; give it a moment.
    ms(250);

    keys(&src);
    assert_eq!(key_frames(&sink, want.len()), want);
    Rig {
        sink,
        _child: child,
        src,
    }
}

#[test]
#[ignore = "needs root and /dev/uinput; run with `just e2e`"]
fn defer_release_keeps_a_key_held_through_a_bounce() {
    relays(
        "defer",
        &[],
        |src| {
            src.write(&key(KEY_MACRO1, 1)).unwrap();
            ms(2);
            src.write(&key(KEY_MACRO1, 0)).unwrap(); // bounce up
            ms(3);
            src.write(&key(KEY_MACRO1, 1)).unwrap(); // bounce down
            ms(80);
            src.write(&key(KEY_MACRO1, 0)).unwrap();
            // Its release goes out about threshold + 1 ms later.
            ms(100);
            src.write(&key(KEY_MACRO2, 1)).unwrap();
            ms(2);
            src.write(&key(KEY_MACRO2, 0)).unwrap();
        },
        &[
            frame(KEY_MACRO1, 1),
            deferred_up(KEY_MACRO1),
            frame(KEY_MACRO2, 1),
            deferred_up(KEY_MACRO2),
        ],
    );
}

#[test]
#[ignore = "needs root and /dev/uinput; run with `just e2e`"]
fn drop_press_filters_chatter_and_relays_leds() {
    let Rig { src, sink, _child } = relays(
        "drop",
        &["--no-defer-release"],
        |src| {
            src.write(&key(KEY_MACRO1, 1)).unwrap();
            ms(2);
            src.write(&key(KEY_MACRO1, 0)).unwrap();
            ms(5);
            src.write(&key(KEY_MACRO1, 1)).unwrap(); // chatter
            ms(2);
            src.write(&key(KEY_MACRO1, 0)).unwrap(); // its release
            ms(60);
            src.write(&key(KEY_MACRO1, 1)).unwrap();
            ms(2);
            src.write(&key(KEY_MACRO1, 0)).unwrap();
            src.write(&key(KEY_MACRO2, 1)).unwrap(); // another key right away
            src.write(&key(KEY_MACRO2, 0)).unwrap();
        },
        &[
            frame(KEY_MACRO1, 1),
            frame(KEY_MACRO1, 0),
            frame(KEY_MACRO1, 1),
            frame(KEY_MACRO1, 0),
            frame(KEY_MACRO2, 1),
            frame(KEY_MACRO2, 0),
        ],
    );

    // Caps Lock set on the virtual keyboard reaches the real one.
    sink.write(&[
        InputEvent::new(EV_LED, LED_CAPSL, 1),
        InputEvent::syn_report(),
    ])
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut buf = [InputEvent::default(); 64];
    let mut seen = false;
    while !seen
        && readable(
            src.as_raw_fd(),
            deadline.saturating_duration_since(Instant::now()),
        )
    {
        let n = src.read(&mut buf).unwrap();
        seen = buf[..n]
            .iter()
            .any(|e| (e.type_, e.code, e.value) == (EV_LED, LED_CAPSL, 1));
    }
    sink.write(&[
        InputEvent::new(EV_LED, LED_CAPSL, 0),
        InputEvent::syn_report(),
    ])
    .unwrap();
    assert!(seen, "LED change never reached the source keyboard");
}
