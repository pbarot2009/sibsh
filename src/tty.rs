//! Terminal introspection without external crates.
//!
//! The window size is read with `ioctl(TIOCGWINSZ)` — a single syscall, safe
//! to run before every editor repaint (the previous implementation forked
//! `stty size`, which costs a process spawn per query). When the ioctl fails
//! (non-terminal stdin, exotic platforms) the code falls back to parsing
//! `stty size` output, then to a 24×80 default.

use std::ffi::{c_int, c_ulong};
use std::process::Command;

#[repr(C)]
#[derive(Default)]
#[allow(clippy::struct_field_names)] // mirrors the C `struct winsize`
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

// TIOCGWINSZ request codes from the platform C headers
// (asm-generic/ioctls.h on Linux, sys/ioctl.h on the BSDs and macOS).
#[cfg(target_os = "linux")]
const TIOCGWINSZ: c_ulong = 0x5413;
#[cfg(not(target_os = "linux"))]
const TIOCGWINSZ: c_ulong = 0x4008_7468;

unsafe extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
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
