# mechabounce

Keyboard chatter filter for Linux. A worn mechanical switch sometimes
registers one press as two. mechabounce grabs the keyboard's evdev device and
drops any press that comes within a threshold of that key's last release. It
re-emits everything else on a virtual copy of the keyboard through uinput. It
works below X11 and Wayland, so it covers every desktop and the console.

It's a rewrite of [KeyboardChatteringFix-Linux][kcf] as a small Rust binary:
about 400 KB, under 2 MB of memory, two syscalls per keystroke, and no
dependency besides libc.

[kcf]: https://github.com/finkrer/KeyboardChatteringFix-Linux

## The rule

A press is chatter if it comes no more than `--threshold` ms (default 30)
after the same key's last release. The release that belongs to a dropped
press is dropped as well. Dropped presses don't restart the clock, so a burst
of bounces can't lock a key out. Deliberate fast double taps leave about
50 ms between release and press, so the default stays below that.

The virtual keyboard has the real one's name and IDs, so per-device desktop
settings still apply. Caps Lock and other LED changes made on the virtual
keyboard are passed back to the real one.

## Install

Needs Rust and [just](https://github.com/casey/just).

```sh
mechabounce --list                     # find your keyboard (or: ls /dev/input/by-id)
just run usb-Your_Keyboard-event-kbd   # try it in the foreground (sudo)
```

To run it as a service, put your keyboard's `ID_SERIAL` in
`dist/70-mechabounce.rules` (the file says how to find it), then:

```sh
just install
```

This installs `/usr/local/bin/mechabounce`, a `mechabounce@.service` template
and the udev rule. udev starts one instance per keyboard interface when the
keyboard is plugged in, and systemd stops it on unplug. The service runs as
root for its two device nodes, but with no capabilities, a read-only system,
and no network.
Set the threshold and other options in `/etc/mechabounce.conf`, and follow the
log with `journalctl -u 'mechabounce@*' -f`.

## Options

```
-k, --keyboard DEV   a name in /dev/input/by-id, an eventN name, or a path
-t, --threshold MS   chatter threshold [default: 30]
-v, --verbosity N    0 quiet, 1 log filtered events, 2 log every key [default: 1]
-l, --list           list keyboards in /dev/input/by-id
```

## Development

```sh
just check   # fmt, clippy, unit tests
just e2e     # through the real kernel: fake uinput keyboard -> mechabounce -> virtual keyboard (sudo)
```
