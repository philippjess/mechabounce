//! Raw evdev and uinput access: the kernel structs, ioctl numbers and thin
//! wrappers over read/write/ioctl. No libevdev.

use std::ffi::CStr;
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::{size_of, size_of_val};
use std::os::fd::{AsRawFd, RawFd};
use std::path::Path;

use libc::{c_int, c_long, c_ulong};

// linux/input-event-codes.h
pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_REL: u16 = 0x02;
pub const EV_ABS: u16 = 0x03;
pub const EV_MSC: u16 = 0x04;
pub const EV_SW: u16 = 0x05;
pub const EV_LED: u16 = 0x11;
pub const EV_SND: u16 = 0x12;
pub const EV_REP: u16 = 0x14;
pub const EV_FF: u16 = 0x15;
pub const EV_CNT: usize = 0x20;

pub const SYN_REPORT: u16 = 0;
pub const SYN_DROPPED: u16 = 3;
pub const MSC_SCAN: u16 = 4;
pub const REP_DELAY: u16 = 0;
pub const REP_PERIOD: u16 = 1;
pub const LED_CAPSL: u16 = 1;

pub const KEY_CNT: usize = 0x300;
const REL_CNT: usize = 0x10;
const ABS_CNT: usize = 0x40;
const MSC_CNT: usize = 0x08;
const SW_CNT: usize = 0x11;
const LED_CNT: usize = 0x10;
const SND_CNT: usize = 0x08;
const PROP_CNT: usize = 0x20;

pub const BUS_VIRTUAL: u16 = 0x06;

const ULONG_BITS: usize = c_ulong::BITS as usize;

/// Key state bitmap in the kernel's layout (an array of `unsigned long`).
pub type KeyBits = [c_ulong; KEY_CNT / ULONG_BITS];

pub fn test_bit(words: &[c_ulong], i: usize) -> bool {
    words
        .get(i / ULONG_BITS)
        .is_some_and(|w| (w >> (i % ULONG_BITS)) & 1 == 1)
}

pub fn set_bit(words: &mut [c_ulong], i: usize, on: bool) {
    if let Some(w) = words.get_mut(i / ULONG_BITS) {
        let mask = 1 << (i % ULONG_BITS);
        if on { *w |= mask } else { *w &= !mask }
    }
}

pub fn ones(words: &[c_ulong]) -> impl Iterator<Item = usize> + '_ {
    (0..words.len() * ULONG_BITS).filter(|&i| test_bit(words, i))
}

/// `struct input_event`. The kernel's 64-bit-time layout on 32-bit targets
/// uses `__kernel_ulong_t` seconds, which is `long`-sized as well.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputEvent {
    pub sec: c_long,
    pub usec: c_long,
    pub type_: u16,
    pub code: u16,
    pub value: i32,
}

impl InputEvent {
    /// An event without a timestamp; the kernel stamps what we write.
    pub const fn new(type_: u16, code: u16, value: i32) -> Self {
        Self {
            sec: 0,
            usec: 0,
            type_,
            code,
            value,
        }
    }

    pub const fn syn_report() -> Self {
        Self::new(EV_SYN, SYN_REPORT, 0)
    }

    pub fn time_ns(&self) -> u64 {
        self.sec as u64 * 1_000_000_000 + self.usec as u64 * 1_000
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct InputId {
    pub bustype: u16,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct AbsInfo {
    pub value: i32,
    pub minimum: i32,
    pub maximum: i32,
    pub fuzz: i32,
    pub flat: i32,
    pub resolution: i32,
}

#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

#[repr(C)]
struct UinputAbsSetup {
    code: u16,
    absinfo: AbsInfo,
}

// asm-generic _IOC layout (x86, arm, riscv): dir:2 | size:14 | type:8 | nr:8.
#[cfg(any(
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "mips",
    target_arch = "mips64",
    target_arch = "sparc",
    target_arch = "sparc64"
))]
compile_error!("this architecture's ioctl numbers don't use the asm-generic layout");
const fn ioc(dir: u64, ty: u8, nr: u8, size: usize) -> u64 {
    (dir << 30) | ((size as u64) << 16) | ((ty as u64) << 8) | nr as u64
}
const fn iow(ty: u8, nr: u8, size: usize) -> u64 {
    ioc(1, ty, nr, size)
}
const fn ior(ty: u8, nr: u8, size: usize) -> u64 {
    ioc(2, ty, nr, size)
}

const EVIOCGID: u64 = ior(b'E', 0x02, size_of::<InputId>());
const EVIOCGREP: u64 = ior(b'E', 0x03, size_of::<[u32; 2]>());
const fn eviocgname(len: usize) -> u64 {
    ior(b'E', 0x06, len)
}
const fn eviocgprop(len: usize) -> u64 {
    ior(b'E', 0x09, len)
}
const fn eviocgkey(len: usize) -> u64 {
    ior(b'E', 0x18, len)
}
const fn eviocgbit(ev: u16, len: usize) -> u64 {
    ior(b'E', 0x20 + ev as u8, len)
}
const fn eviocgabs(abs: u16) -> u64 {
    ior(b'E', 0x40 + abs as u8, size_of::<AbsInfo>())
}
const EVIOCGRAB: u64 = iow(b'E', 0x90, size_of::<c_int>());
const EVIOCSCLOCKID: u64 = iow(b'E', 0xa0, size_of::<c_int>());

const UI_DEV_CREATE: u64 = ioc(0, b'U', 1, 0);
const UI_DEV_SETUP: u64 = iow(b'U', 3, size_of::<UinputSetup>());
const UI_ABS_SETUP: u64 = iow(b'U', 4, size_of::<UinputAbsSetup>());
const UI_SET_EVBIT: u64 = iow(b'U', 100, size_of::<c_int>());
const UI_SET_KEYBIT: u64 = iow(b'U', 101, size_of::<c_int>());
const UI_SET_RELBIT: u64 = iow(b'U', 102, size_of::<c_int>());
const UI_SET_ABSBIT: u64 = iow(b'U', 103, size_of::<c_int>());
const UI_SET_MSCBIT: u64 = iow(b'U', 104, size_of::<c_int>());
const UI_SET_LEDBIT: u64 = iow(b'U', 105, size_of::<c_int>());
const UI_SET_SNDBIT: u64 = iow(b'U', 106, size_of::<c_int>());
const UI_SET_SWBIT: u64 = iow(b'U', 109, size_of::<c_int>());
const UI_SET_PROPBIT: u64 = iow(b'U', 110, size_of::<c_int>());
const fn ui_get_sysname(len: usize) -> u64 {
    ior(b'U', 44, len)
}

fn ioctl(fd: RawFd, req: u64, arg: c_ulong) -> io::Result<c_int> {
    let r = unsafe { libc::ioctl(fd, req as _, arg) };
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

fn ioctl_ptr<T>(fd: RawFd, req: u64, p: *mut T) -> io::Result<c_int> {
    ioctl(fd, req, p as c_ulong)
}

fn read_events(fd: RawFd, buf: &mut [InputEvent]) -> io::Result<usize> {
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), size_of_val(buf)) };
        if n >= 0 {
            return Ok(n as usize / size_of::<InputEvent>());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

fn write_events(fd: RawFd, evs: &[InputEvent]) -> io::Result<()> {
    loop {
        let n = unsafe { libc::write(fd, evs.as_ptr().cast(), size_of_val(evs)) };
        if n >= 0 {
            return if n as usize == size_of_val(evs) {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "short event write",
                ))
            };
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

pub fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// An `/dev/input/eventN` node, opened read/write so LED state can be set.
pub struct Evdev {
    file: File,
}

impl Evdev {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        Ok(Self { file })
    }

    pub fn name(&self) -> io::Result<Vec<u8>> {
        let mut buf = [0u8; 80];
        ioctl_ptr(
            self.as_raw_fd(),
            eviocgname(buf.len() - 1),
            buf.as_mut_ptr(),
        )?;
        Ok(CStr::from_bytes_until_nul(&buf)
            .map_or(&buf[..79], CStr::to_bytes)
            .to_vec())
    }

    pub fn id(&self) -> io::Result<InputId> {
        let mut id = InputId::default();
        ioctl_ptr(self.as_raw_fd(), EVIOCGID, &mut id)?;
        Ok(id)
    }

    /// Supported codes of event type `ev`, or the supported types for `ev == 0`.
    pub fn bits(&self, ev: u16, cnt: usize) -> io::Result<Vec<c_ulong>> {
        let mut words = vec![0 as c_ulong; cnt.div_ceil(ULONG_BITS)];
        ioctl_ptr(
            self.as_raw_fd(),
            eviocgbit(ev, size_of_val(&words[..])),
            words.as_mut_ptr(),
        )?;
        Ok(words)
    }

    pub fn props(&self) -> io::Result<Vec<c_ulong>> {
        let mut words = vec![0 as c_ulong; PROP_CNT.div_ceil(ULONG_BITS)];
        ioctl_ptr(
            self.as_raw_fd(),
            eviocgprop(size_of_val(&words[..])),
            words.as_mut_ptr(),
        )?;
        Ok(words)
    }

    /// Keys the kernel currently considers held.
    pub fn key_state(&self, out: &mut KeyBits) -> io::Result<()> {
        ioctl_ptr(
            self.as_raw_fd(),
            eviocgkey(size_of_val(out)),
            out.as_mut_ptr(),
        )?;
        Ok(())
    }

    pub fn abs_info(&self, code: u16) -> io::Result<AbsInfo> {
        let mut info = AbsInfo::default();
        ioctl_ptr(self.as_raw_fd(), eviocgabs(code), &mut info)?;
        Ok(info)
    }

    /// Autorepeat `[delay, period]` in milliseconds.
    pub fn repeat(&self) -> io::Result<[u32; 2]> {
        let mut rep = [0u32; 2];
        ioctl_ptr(self.as_raw_fd(), EVIOCGREP, rep.as_mut_ptr())?;
        Ok(rep)
    }

    /// While grabbed, only this fd sees the device's events.
    pub fn grab(&self, on: bool) -> io::Result<()> {
        ioctl(self.as_raw_fd(), EVIOCGRAB, on as c_ulong)?;
        Ok(())
    }

    /// Stamp events with CLOCK_MONOTONIC instead of the wall clock, so NTP
    /// steps can't distort the gaps we measure.
    pub fn use_monotonic_clock(&self) -> io::Result<()> {
        let mut clock: c_int = libc::CLOCK_MONOTONIC;
        ioctl_ptr(self.as_raw_fd(), EVIOCSCLOCKID, &mut clock)?;
        Ok(())
    }

    /// Blocks until at least one event is available; returns the event count.
    pub fn read(&self, buf: &mut [InputEvent]) -> io::Result<usize> {
        read_events(self.as_raw_fd(), buf)
    }

    pub fn write(&self, evs: &[InputEvent]) -> io::Result<()> {
        write_events(self.as_raw_fd(), evs)
    }
}

impl AsRawFd for Evdev {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

/// Declares the capabilities of a uinput device before it is created.
pub struct UinputBuilder {
    file: File,
}

impl UinputBuilder {
    pub fn new() -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/uinput")?;
        Ok(Self { file })
    }

    pub fn enable_type(&self, ty: u16) -> io::Result<()> {
        ioctl(self.file.as_raw_fd(), UI_SET_EVBIT, ty.into())?;
        Ok(())
    }

    /// Enables one code of a type already enabled with [`Self::enable_type`].
    /// Absolute axes go through [`Self::enable_abs`] instead.
    pub fn enable_code(&self, ty: u16, code: u16) -> io::Result<()> {
        let req = match ty {
            EV_KEY => UI_SET_KEYBIT,
            EV_REL => UI_SET_RELBIT,
            EV_MSC => UI_SET_MSCBIT,
            EV_SW => UI_SET_SWBIT,
            EV_LED => UI_SET_LEDBIT,
            EV_SND => UI_SET_SNDBIT,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("event type {ty:#x} has no code bits"),
                ));
            }
        };
        ioctl(self.file.as_raw_fd(), req, code.into())?;
        Ok(())
    }

    pub fn enable_abs(&self, code: u16, absinfo: AbsInfo) -> io::Result<()> {
        ioctl(self.file.as_raw_fd(), UI_SET_ABSBIT, code.into())?;
        let mut setup = UinputAbsSetup { code, absinfo };
        ioctl_ptr(self.file.as_raw_fd(), UI_ABS_SETUP, &mut setup)?;
        Ok(())
    }

    pub fn enable_prop(&self, prop: u16) -> io::Result<()> {
        ioctl(self.file.as_raw_fd(), UI_SET_PROPBIT, prop.into())?;
        Ok(())
    }

    pub fn create(self, id: InputId, name: &[u8]) -> io::Result<Uinput> {
        let mut setup = UinputSetup {
            id,
            name: [0; 80],
            ff_effects_max: 0,
        };
        let n = name.len().min(setup.name.len() - 1);
        setup.name[..n].copy_from_slice(&name[..n]);
        ioctl_ptr(self.file.as_raw_fd(), UI_DEV_SETUP, &mut setup)?;
        ioctl(self.file.as_raw_fd(), UI_DEV_CREATE, 0)?;
        Ok(Uinput { file: self.file })
    }
}

/// A live uinput device. Closing it removes the device and releases its keys.
pub struct Uinput {
    file: File,
}

impl Uinput {
    /// A virtual twin of `src`: same name, ids, capabilities and repeat
    /// settings, so per-device desktop settings keep applying. Force feedback
    /// is left out, since we couldn't play the effects on the source's behalf.
    pub fn clone_of(src: &Evdev) -> io::Result<Self> {
        let b = UinputBuilder::new()?;
        let types = src.bits(0, EV_CNT)?;
        for ty in ones(&types).map(|t| t as u16).filter(|&t| t != EV_FF) {
            b.enable_type(ty)?;
            let cnt = match ty {
                EV_KEY => KEY_CNT,
                EV_REL => REL_CNT,
                EV_ABS => ABS_CNT,
                EV_MSC => MSC_CNT,
                EV_SW => SW_CNT,
                EV_LED => LED_CNT,
                EV_SND => SND_CNT,
                _ => continue,
            };
            for code in ones(&src.bits(ty, cnt)?).map(|c| c as u16) {
                if ty == EV_ABS {
                    b.enable_abs(code, src.abs_info(code)?)?;
                } else {
                    b.enable_code(ty, code)?;
                }
            }
        }
        for prop in ones(&src.props()?) {
            b.enable_prop(prop as u16)?;
        }
        let ui = b.create(src.id()?, &src.name()?)?;
        if test_bit(&types, EV_REP.into()) {
            // The input core autorepeats on the virtual device; match its timing.
            let [delay, period] = src.repeat()?;
            ui.write(&[
                InputEvent::new(EV_REP, REP_DELAY, delay as i32),
                InputEvent::new(EV_REP, REP_PERIOD, period as i32),
            ])?;
        }
        Ok(ui)
    }

    /// The `inputN` name under /sys/class/input.
    pub fn sysname(&self) -> io::Result<String> {
        let mut buf = [0u8; 64];
        ioctl_ptr(
            self.as_raw_fd(),
            ui_get_sysname(buf.len()),
            buf.as_mut_ptr(),
        )?;
        let name = CStr::from_bytes_until_nul(&buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(name.to_string_lossy().into_owned())
    }

    /// Events sent *to* the device by its users, such as LED changes.
    pub fn read(&self, buf: &mut [InputEvent]) -> io::Result<usize> {
        read_events(self.as_raw_fd(), buf)
    }

    pub fn write(&self, evs: &[InputEvent]) -> io::Result<()> {
        write_events(self.as_raw_fd(), evs)
    }
}

impl AsRawFd for Uinput {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}
