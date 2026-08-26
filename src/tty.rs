//! Terminal introspection and mode control without external crates.
//!
//! The window size is read with `ioctl(TIOCGWINSZ)` — a single syscall, safe
//! to run before every editor repaint (the previous implementation forked
//! `stty size`, which costs a process spawn per query). When the ioctl fails
//! (non-terminal stdin, exotic platforms) the code falls back to parsing
//! `stty size` output, then to a 24×80 default.
//!
//! Raw mode is likewise set with `tcgetattr`/`tcsetattr` directly rather
//! than by shelling out to `stty raw -echo` / `stty sane`. Spawning `stty`
//! forks and execs a child process that must itself attach to the
//! controlling terminal and complete its own `ioctl` before the mode change
//! is actually in effect; a keystroke landing in that window is echoed once
//! by the kernel's line discipline (still in cooked mode) and then again by
//! the editor's own repaint once raw mode finally lands, which is what
//! produced the intermittent duplicated/ghosted prompt lines. A direct
//! syscall has no such window.

use std::ffi::{c_int, c_uchar, c_uint, c_ulong};
use std::io;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

#[repr(C)]
#[derive(Default)]
#[allow(clippy::struct_field_names)] // mirrors the C `struct winsize`
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

// `struct termios` layout per POSIX/glibc `<bits/termios-struct.h>`. The
// four mode-flag fields are `tcflag_t` (`unsigned int`, 4 bytes) — using
// `c_ulong` (8 bytes on 64-bit Linux) here would misalign every field after
// the first and corrupt whatever `tcsetattr` reads back. `NCCS` is 32 on
// Linux; macOS/BSD use 20, but this struct is only ever populated by our
// own `tcgetattr` call and read back by our own `tcsetattr` — never handed
// to a differently-compiled binary — so an oversized `c_cc` is harmless as
// long as the leading fields line up, which they do on both.
#[repr(C)]
#[derive(Clone, Copy)]
struct Termios {
    c_iflag: c_uint,
    c_oflag: c_uint,
    c_cflag: c_uint,
    c_lflag: c_uint,
    c_line: c_uchar,
    c_cc: [c_uchar; 32],
    c_ispeed: c_uint,
    c_ospeed: c_uint,
}

impl Default for Termios {
    fn default() -> Self {
        // SAFETY: an all-zero `Termios` is a valid bit pattern (all fields
        // are plain integers/arrays); it is only ever used as an out-param
        // for `tcgetattr` before being read.
        unsafe { std::mem::zeroed() }
    }
}

// Flag bits and control-character indices from `<bits/termios-c_*.h>` /
// `<sys/termios.h>`. `ICANON`/`ECHO`/`ISIG`/`ICRNL`/`IXON`/`OPOST` and the
// `VMIN`/`VTIME` indices happen to share the same values across Linux and
// the BSDs/macOS (all descend from the same historical termio layout), so
// one set of constants covers every Unix target this project builds for —
// unlike the raw `ioctl` request codes below, which do differ per OS.
mod bits {
    use std::ffi::c_uint;
    pub const ICANON: c_uint = 0o0000002;
    pub const ECHO: c_uint = 0o0000010;
    pub const ISIG: c_uint = 0o0000001;
    pub const ICRNL: c_uint = 0o0000400;
    pub const IXON: c_uint = 0o0002000;
    pub const OPOST: c_uint = 0o0000001;
    pub const VMIN: usize = 6;
    pub const VTIME: usize = 5;
    pub const TCSANOW: i32 = 0;
}

// TIOCGWINSZ request code from the platform C headers
// (asm-generic/ioctls.h on Linux, sys/ioctl.h on the BSDs and macOS).
#[cfg(target_os = "linux")]
const TIOCGWINSZ: c_ulong = 0x5413;
#[cfg(not(target_os = "linux"))]
const TIOCGWINSZ: c_ulong = 0x4008_7468;

unsafe extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    // Real libc functions (POSIX `<termios.h>`), not raw ioctls — glibc,
    // musl, and the BSD/macOS libcs all export these directly, so no
    // platform-specific request-code table is needed here the way
    // `TIOCGWINSZ` above needs one.
    fn tcgetattr(fd: c_int, termios: *mut Termios) -> c_int;
    fn tcsetattr(fd: c_int, action: c_int, termios: *const Termios) -> c_int;
}

/// Whether [`enter_raw_mode`] has installed a saved-state guard this
/// process still owns. Used only to decide whether [`restore_on_exit`]
/// (called from a panic hook / signal path) has anything to do; the actual
/// saved settings live in [`enter_raw_mode`]'s caller-owned [`RawGuard`].
static RAW_ACTIVE: AtomicBool = AtomicBool::new(false);

/// RAII guard restoring the terminal's original mode when dropped — on
/// the normal return path, but also on an early return, a `?`, or a panic
/// unwinding through the caller's stack frame. This is what closes the
/// "raw mode terminal state resets on exit" gap: previously only a
/// clean, panic-free return from `read_line_raw` ran the cleanup.
pub struct RawGuard {
    original: Termios,
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        // SAFETY: `self.original` was populated by a prior successful
        // `tcgetattr` call in `enter_raw_mode`; fd 0 is stdin, valid for
        // the life of the process.
        unsafe {
            tcsetattr(0, bits::TCSANOW, &self.original);
        }
        RAW_ACTIVE.store(false, Ordering::SeqCst);
    }
}

/// Puts stdin into raw mode (no echo, no line buffering, no signal-
/// generating control characters so Ctrl+C/Ctrl+Z reach the editor as
/// plain bytes) and returns a guard that restores the previous mode when
/// dropped. Returns `None` when stdin is not a terminal (piped scripts,
/// tests), matching the old `stty raw -echo` failing in the same case.
pub fn enter_raw_mode() -> Option<RawGuard> {
    let mut original = Termios::default();
    // SAFETY: `original` is a valid, exclusively borrowed out-param.
    if unsafe { tcgetattr(0, &mut original) } != 0 {
        return None;
    }
    let mut raw = original;
    raw.c_lflag &= !(bits::ICANON | bits::ECHO | bits::ISIG);
    raw.c_iflag &= !(bits::ICRNL | bits::IXON);
    raw.c_oflag &= !bits::OPOST;
    raw.c_cc[bits::VMIN] = 1;
    raw.c_cc[bits::VTIME] = 0;
    // SAFETY: `raw` is a fully-initialized `Termios` derived from a
    // successful `tcgetattr`; fd 0 is stdin.
    if unsafe { tcsetattr(0, bits::TCSANOW, &raw) } != 0 {
        return None;
    }
    RAW_ACTIVE.store(true, Ordering::SeqCst);
    Some(RawGuard { original })
}

/// Best-effort terminal restore for use from a panic hook: if raw mode is
/// currently active, falls back to `stty sane` (a subprocess is acceptable
/// here since this only runs once, right before the process dies, not on
/// every keystroke). Idempotent and safe to call even when not in raw mode.
pub fn restore_on_exit() {
    if RAW_ACTIVE.swap(false, Ordering::SeqCst) {
        let _ = Command::new("stty")
            .arg("sane")
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// Wraps a body that runs with stdin in raw mode, guaranteeing the
/// original mode is restored via [`RawGuard`]'s `Drop` even if `body`
/// returns early or an inner `?` propagates an error — the two cases the
/// previous `stty raw -echo` / `stty sane` pair around `read_line_raw`
/// did not cover.
pub fn with_raw_mode<T>(body: impl FnOnce() -> io::Result<T>) -> Option<io::Result<T>> {
    let _guard = enter_raw_mode()?;
    Some(body())
}

/// `(rows, cols)` of the terminal attached to stdin. Falls back to parsing
/// `stty size`, then to `(24, 80)`.
pub fn terminal_size() -> (usize, usize) {
    // SAFETY: `ws` is a valid, exclusively borrowed `Winsize`; TIOCGWINSZ
    // only writes through the given pointer.
    unsafe {
        let mut ws = Winsize::default();
        if ioctl(0, TIOCGWINSZ, &mut ws) == 0 && ws.ws_row > 0 && ws.ws_col > 0 {
            return (usize::from(ws.ws_row), usize::from(ws.ws_col));
        }
    }
    stty_size().unwrap_or((24, 80))
}

/// Terminal width in columns; convenience wrapper over [`terminal_size`].
pub fn terminal_width() -> usize {
    terminal_size().1
}

fn stty_size() -> Option<(usize, usize)> {
    use std::process::Stdio;
    let output = Command::new("stty")
        .arg("size")
        .stderr(Stdio::null()) // non-terminal stdin: expected failure
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut values = text.split_whitespace();
    let rows = values.next()?.parse().ok()?;
    let cols = values.next()?.parse().ok()?;
    Some((rows, cols))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_size_returns_sane_values() {
        let (rows, cols) = terminal_size();
        assert!(rows > 0);
        assert!(cols > 0);
        // A pty-less test runner still gets *something* plausible.
        assert!(rows < 10_000 && cols < 10_000);
    }
}
