//! Interactive line reading: tab completion, basic line editing, and
//! history navigation. Raw terminal mode is entered via direct
//! `tcgetattr`/`tcsetattr` calls in [`crate::tty`] (no `stty` subprocess,
//! no dependencies). When stdin is not a terminal (scripts, tests), falls
//! back to plain buffered line reading.

use crate::builtins::Builtins;
use crate::config::expand_tilde;
use crate::tty;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

const CTRL_A: u8 = 0x01;
const CTRL_C: u8 = 0x03;
const CTRL_D: u8 = 0x04;
const CTRL_E: u8 = 0x05;
const BACKSPACE: u8 = 0x7f;

/// Result of completing the word before the cursor.
#[derive(Debug, Clone)]
pub struct Completion {
    /// Text to insert at the cursor: the part of the shared candidate prefix
    /// that extends beyond what the user already typed. Empty when there is
    /// nothing unambiguous to insert.
    pub insert: String,
    /// All matching candidates (for display on a second Tab press).
    pub candidates: Vec<String>,
}

/// Completes the token ending at the end of `line`.
///
/// First word: builtins, aliases, then `$PATH` executables.
/// Any other word: filesystem paths relative to the token's directory.
pub fn complete(line: &str, aliases: &[(String, String)]) -> Completion {
    let dirs: Vec<PathBuf> = env::var("PATH")
        .map(|p| env::split_paths(&p).collect())
        .unwrap_or_default();
    complete_in(line, aliases, &dirs)
}

/// Like [`complete`], but searches only the given directories for executables.
/// Tests pass an empty or synthetic list so results never depend on the
/// machine's real `$PATH`.
pub fn complete_in(line: &str, aliases: &[(String, String)], path_dirs: &[PathBuf]) -> Completion {
    let token_start = line.rfind([' ', '\t']).map_or(0, |idx| idx + 1);
    let token = &line[token_start..];

    let mut candidates = if token_start == 0 {
        command_candidates(token, aliases, path_dirs)
    } else {
        path_candidates(token)
    };
    candidates.sort();
    candidates.dedup();

    let shared = common_prefix(&candidates);

    let insert = match candidates.as_slice() {
        [single] => {
            let mut insert = if shared.chars().count() > token.chars().count() {
                shared[token.len()..].to_string()
            } else {
                String::new()
            };
            // A fully completed first word gets a trailing space. Directory
            // paths keep their `/` so the next Tab completes deeper.
            if shared == *single && !single.ends_with('/') && token_start == 0 {
                insert.push(' ');
            }
            insert
        }
        _ => String::new(),
    };

    Completion { insert, candidates }
}

/// Builtins + aliases + executables found in the given directories starting
/// with `prefix`.
fn command_candidates(
    prefix: &str,
    aliases: &[(String, String)],
    path_dirs: &[PathBuf],
) -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();

    for name in Builtins::NAMES {
        if name.starts_with(prefix) {
            candidates.push(name.to_string());
        }
    }
    for (name, _) in aliases {
        if name.starts_with(prefix) && !candidates.contains(name) {
            candidates.push(name.clone());
        }
    }

    for dir in path_dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(prefix)
                && !candidates.contains(&name)
                && is_executable(&entry.path())
            {
                candidates.push(name);
            }
        }
    }

    candidates
}

/// Directory entries matching `token`, treated as a partial path.
fn path_candidates(token: &str) -> Vec<String> {
    let expanded = expand_tilde(token);
    let (dir_str, file_part) = match expanded.rfind('/') {
        Some(idx) => (&expanded[..=idx], &expanded[idx + 1..]),
        None => ("", expanded.as_str()),
    };

    let dir = match dir_str.strip_suffix('/') {
        Some("") => PathBuf::from("/"),
        Some(stripped) => PathBuf::from(stripped),
        None => PathBuf::from("."),
    };

    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let show_hidden = file_part.starts_with('.');
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(file_part) {
            continue;
        }
        if name.starts_with('.') && !show_hidden {
            continue;
        }
        let suffix = if entry.path().is_dir() { "/" } else { "" };
        candidates.push(format!("{dir_str}{name}{suffix}"));
    }
    candidates
}

/// Longest common prefix of all candidates ("" when empty).
fn common_prefix(candidates: &[String]) -> String {
    let Some(first) = candidates.first() else {
        return String::new();
    };
    let mut len = first.len();
    for candidate in &candidates[1..] {
        len = len.min(candidate.len());
        while first.as_bytes()[..len] != candidate.as_bytes()[..len] {
            len -= 1;
        }
    }
    // Do not cut a multi-byte character in half.
    while len > 0 && !first.is_char_boundary(len) {
        len -= 1;
    }
    first[..len].to_string()
}

fn is_executable(path: &std::path::Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// Reads one input line interactively. Returns `Ok(None)` on EOF (Ctrl+D on
/// an empty line). Falls back to plain buffered reading when stdin is not a
/// terminal.
///
/// Raw mode is entered and left via [`tty::with_raw_mode`], which uses
/// `tcgetattr`/`tcsetattr` directly instead of forking `stty raw -echo` /
/// `stty sane`. The guard it installs restores the original mode on every
/// exit path out of `read_line_raw` — normal return, an `Err` from a `?`,
/// or a panic unwinding through this frame — which a bare pair of `stty`
/// subprocess calls around the function body did not: an early return or
/// panic used to skip the trailing `stty sane` entirely, leaving the
/// user's terminal stuck in raw/no-echo mode after sibsh exited.
pub fn read_line(
    prompt: &str,
    history: &[String],
    aliases: &[(String, String)],
) -> io::Result<Option<String>> {
    // `with_raw_mode` returns `None` when stdin is not a TTY (piped
    // scripts, tests), which selects the non-interactive fallback.
    match tty::with_raw_mode(|| read_line_raw(prompt, history, aliases)) {
        Some(result) => result,
        None => read_line_plain(prompt),
    }
}

fn read_line_plain(prompt: &str) -> io::Result<Option<String>> {
    // The line reader owns prompt painting in both modes; the REPL never
    // prints the prompt itself (same model as zsh's ZLE and fish).
    print!("{prompt}");
    let _ = io::stdout().flush();

    let mut line = String::new();
    match io::stdin().read_line(&mut line)? {
        0 => Ok(None),
        _ => Ok(Some(line)),
    }
}

/// One step of ANSI-escape-sequence recognition, shared by [`display_width`]
/// and [`render_rows`] so both treat sequences identically.
///
/// Correctly distinguishes the CSI *introducer* from a CSI *final byte*:
/// `ESC [` moves into the CSI-parameter state, and only a byte in the
/// 0x40..=0x7E final-byte range seen *there* closes the sequence. A prior
/// version conflated the two — it fed every byte seen while "in ANSI" to
/// the same 0x40..=0x7E check, and `[` (0x5B) is itself inside that range,
/// so the introducer immediately closed the state it had just opened,
/// leaving the two parameter/final bytes of every sequence (e.g. `38` and
/// `m` in `\x1b[38;5;242m`) treated as plain visible text. `AnsiState`
/// makes "just saw ESC" and "inside CSI params" two different states so
/// the introducer can never be mistaken for a final byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnsiState {
    /// Not inside an escape sequence; the next byte is ordinary text
    /// unless it is ESC.
    Normal,
    /// Just consumed ESC; only `[` continues into a CSI, anything else
    /// ends the (two-byte) escape immediately.
    Escaped,
    /// Inside `ESC [ ... `; consuming parameter/intermediate bytes until
    /// a final byte (0x40..=0x7E) closes the sequence.
    Csi,
}

impl AnsiState {
    /// Advances the state machine by one character. Returns `true` when
    /// `ch` is ordinary visible text that the caller should measure —
    /// i.e. it was not part of an escape sequence and did not just
    /// transition into or out of one.
    fn step(&mut self, ch: char) -> bool {
        match (*self, ch) {
            (Self::Normal, '\x1b') => {
                *self = Self::Escaped;
                false
            }
            (Self::Normal, _) => true,
            (Self::Escaped, '[') => {
                *self = Self::Csi;
                false
            }
            (Self::Escaped, _) => {
                // A non-CSI escape (sibsh never emits these, but a custom
                // `prompt` template or pasted input might): the sequence
                // is exactly ESC + this byte.
                *self = Self::Normal;
                false
            }
            (Self::Csi, ch) if is_csi_final_byte(ch) => {
                *self = Self::Normal;
                false
            }
            (Self::Csi, _) => false,
        }
    }
}

/// True for a byte that legally terminates a CSI sequence (`ESC [ ... final`).
/// The final byte is any of `@` through `~` (0x40..=0x7E) per ECMA-48 — not
/// only ASCII letters. Only meaningful once already inside a CSI (see
/// [`AnsiState`]): `[`, the CSI introducer, also falls in this range, so
/// this check must never be applied to the byte immediately after ESC.
fn is_csi_final_byte(ch: char) -> bool {
    matches!(ch as u32, 0x40..=0x7e)
}

/// Visible terminal cell width of `s`, skipping ANSI escape sequences.
/// Wide CJK characters count as 2 cells, nerd-font private-use glyphs as 1.
fn display_width(s: &str) -> usize {
    let mut width = 0;
    let mut state = AnsiState::Normal;
    for ch in s.chars() {
        if state.step(ch) {
            width += char_width(ch);
        }
    }
    width
}

/// Terminal cell width of one character.
fn char_width(ch: char) -> usize {
    let c = ch as u32;
    match c {
        // Control characters and combining/zero-width forms occupy no cells.
        0x00..=0x1f | 0x7f | 0x0300..=0x036f | 0x200b..=0x200f | 0xfe00..=0xfe0f => 0,
        // East Asian wide ranges (common blocks).
        0x1100..=0x115f
        | 0x2e80..=0x303e
        | 0x3041..=0x33ff
        | 0x3400..=0x4dbf
        | 0x4e00..=0x9fff
        | 0xa000..=0xa4cf
        | 0xac00..=0xd7a3
        | 0xf900..=0xfaff
        | 0xfe30..=0xfe4f
        | 0xff00..=0xff60
        | 0xffe0..=0xffe6
        | 0x20000..=0x2fffd
        | 0x30000..=0x3fffd => 2,
        _ => 1,
    }
}

/// Rows the repaint must move the cursor up: over every row of the previous
/// render (edit area plus any candidate listing below it), clamped so a
/// render taller than the screen — which has already scrolled — cannot walk
/// past the top row.
fn up_rows(base_rows: usize, extra_rows: usize, screen_rows: usize) -> usize {
    (base_rows.saturating_sub(1) + extra_rows).min(screen_rows.saturating_sub(1))
}

/// Number of screen rows the render occupies when drawn from column 0 of a
/// `term_cols`-wide terminal, accounting for line wrapping (deferred-wrap
/// semantics, matching xterm and friends).
fn render_rows(prompt: &str, line: &str, term_cols: usize) -> usize {
    let full = format!("{prompt}{line}");
    let mut rows = 1usize;
    let mut col = 0usize;
    let mut state = AnsiState::Normal;
    for ch in full.chars() {
        if !state.step(ch) {
            continue;
        }
        if ch == '\n' {
            rows += 1;
            col = 0;
        } else {
            let cw = char_width(ch);
            if col + cw > term_cols {
                rows += 1;
                col = cw;
            } else {
                col += cw;
            }
        }
    }
    rows
}

/// Owns everything needed to repaint the editing area: the prompt text, the
/// terminal width, and the geometry of the previous render. Repainting moves
/// the cursor to the top row of the previous render, clears to the end of
/// the screen, and rewrites everything — so wrapping, long lines, history
/// swaps, and candidate lists can never leave stale fragments behind.
///
/// The terminal size is re-queried before every paint (a single syscall), so
/// a window resize is detected on the next keystroke: the editor re-measures
/// the region under the new width and redraws with the edited line, cursor
/// position, history state, and pending completions fully preserved.
struct Painter {
    prompt: String,
    term_cols: usize,
    /// Screen rows occupied by the current render (prompt + edited line,
    /// including wrapped rows), measured down to the row holding the cursor.
    base_rows: usize,
    /// Rows appended *below* the edit area since the last full repaint
    /// (completion candidate listings). Cleared together with the next paint.
    extra_rows: usize,
    /// The line content as of the last paint, used to re-measure the region
    /// after a resize re-flowed the screen under a new width.
    last_line: String,
}

impl Painter {
    fn new(prompt: &str, out: &mut impl Write) -> io::Result<Self> {
        let term_cols = tty::terminal_width();
        write!(out, "{}", crlf(prompt))?;
        out.flush()?;
        Ok(Self {
            prompt: prompt.to_string(),
            term_cols,
            base_rows: render_rows(prompt, "", term_cols),
            extra_rows: 0,
            last_line: String::new(),
        })
    }

    fn repaint(&mut self, out: &mut impl Write, chars: &[char], cursor: usize) -> io::Result<()> {
        let line: String = chars.iter().collect();
        let (screen_rows, cols) = tty::terminal_size();
        if cols != self.term_cols {
            // The window was resized: the terminal has already re-flowed the
            // previous render under the new width, so the remembered row
            // count is wrong. Re-measure it for the same content, keeping
            // whatever candidate rows still sit below the edit area.
            self.term_cols = cols;
            self.base_rows = render_rows(&self.prompt, &self.last_line, cols);
        }
        let up = up_rows(self.base_rows, self.extra_rows, screen_rows);
        if up > 0 {
            write!(out, "\x1b[{up}A")?;
        }
        write!(out, "\r\x1b[J{}{line}", crlf(&self.prompt))?;
        // Walk the cursor back from end-of-line to its edit position in
        // display cells (not chars) so multibyte input stays aligned.
        let behind: usize = chars[cursor..].iter().map(|c| char_width(*c)).sum();
        if behind > 0 {
            write!(out, "\x1b[{behind}D")?;
        }
        self.base_rows = render_rows(&self.prompt, &line, self.term_cols);
        self.extra_rows = 0;
        self.last_line = line;
        out.flush()
    }
}

/// In raw mode the kernel does not translate `\n` into carriage-return +
/// linefeed, so every newline the editor emits must carry its own `\r` or
/// text lands at the previous line's ending column.
fn crlf(text: &str) -> String {
    text.replace('\n', "\r\n")
}

fn print_candidates(out: &mut impl Write, candidates: &[String]) -> io::Result<usize> {
    let term_cols = tty::terminal_width();
    let longest = candidates
        .iter()
        .map(|c| display_width(c))
        .max()
        .unwrap_or(0);
    let columns = (term_cols / (longest + 2)).max(1);

    for (idx, candidate) in candidates.iter().enumerate() {
        if idx % columns == 0 {
            write!(out, "\r\n")?;
        }
        write!(out, "{candidate}")?;
        // Pad to the column width with plain spaces so multibyte names
        // stay aligned (the width format specifier counts bytes).
        let pad = longest - display_width(candidate);
        write!(out, "{}  ", " ".repeat(pad))?;
    }
    write!(out, "\r\n")?;
    // Rows written plus the trailing newline: exactly how far the cursor
    // moved down, so the caller knows how far to move it back up.
    Ok(candidates.len().div_ceil(columns) + 1)
}

/// Applies one Tab press: insert the unambiguous completion, or list all
/// candidates on a second Tab. Returns nothing; state is updated in place.
fn handle_tab(
    out: &mut impl Write,
    painter: &mut Painter,
    chars: &mut Vec<char>,
    cursor: &mut usize,
    aliases: &[(String, String)],
    last_was_tab: &mut bool,
) -> io::Result<()> {
    let prefix: String = chars[..*cursor].iter().collect();
    let completion = complete(&prefix, aliases);

    if !completion.insert.is_empty() {
        for ch in completion.insert.chars() {
            chars.insert(*cursor, ch);
            *cursor += 1;
        }
        painter.repaint(out, chars, *cursor)?;
        *last_was_tab = false;
    } else if *last_was_tab && completion.candidates.len() > 1 {
        let rows = print_candidates(out, &completion.candidates)?;
        // The candidate list extends the area that the next repaint must
        // clear.
        painter.extra_rows += rows;
        painter.repaint(out, chars, *cursor)?;
    } else {
        *last_was_tab = true;
    }
    Ok(())
}

/// Raw-mode line editor loop.
fn read_line_raw(
    prompt: &str,
    history: &[String],
    aliases: &[(String, String)],
) -> io::Result<Option<String>> {
    let mut out = io::stdout().lock();
    let mut stdin = io::stdin();
    let mut byte = [0u8; 1];

    let mut chars: Vec<char> = Vec::new();
    let mut cursor: usize = 0;
    // Index into `history`; == history.len() means "not browsing".
    let mut hist_pos: usize = history.len();
    let mut saved_line: Option<String> = None;
    let mut last_was_tab = false;

    let mut painter = Painter::new(prompt, &mut out)?;

    loop {
        if stdin.read(&mut byte)? == 0 {
            write!(out, "\r\n")?;
            out.flush()?;
            return Ok(if chars.is_empty() {
                None
            } else {
                Some(chars.iter().collect())
            });
        }

        match byte[0] {
            b'\r' | b'\n' => {
                write!(out, "\r\n")?;
                out.flush()?;
                return Ok(Some(chars.iter().collect()));
            }
            CTRL_C => {
                // Clear the edited line first, then mark the cancel on a
                // fresh row so nothing stale remains above.
                painter.repaint(&mut out, &[], 0)?;
                write!(out, "^C\r\n")?;
                out.flush()?;
                return Ok(Some(String::new()));
            }
            CTRL_D => {
                if chars.is_empty() {
                    write!(out, "\r\n")?;
                    out.flush()?;
                    return Ok(None);
                }
                if cursor < chars.len() {
                    chars.remove(cursor);
                    painter.repaint(&mut out, &chars, cursor)?;
                }
            }
            BACKSPACE | 0x08 => {
                if cursor > 0 {
                    cursor -= 1;
                    chars.remove(cursor);
                    painter.repaint(&mut out, &chars, cursor)?;
                }
                last_was_tab = false;
            }
            b'\t' => {
                handle_tab(
                    &mut out,
                    &mut painter,
                    &mut chars,
                    &mut cursor,
                    aliases,
                    &mut last_was_tab,
                )?;
            }
            CTRL_A => {
                cursor = 0;
                painter.repaint(&mut out, &chars, cursor)?;
                last_was_tab = false;
            }
            CTRL_E => {
                cursor = chars.len();
                painter.repaint(&mut out, &chars, cursor)?;
                last_was_tab = false;
            }
            0x1b => {
                handle_escape(
                    &mut stdin,
                    &mut byte,
                    &mut chars,
                    &mut cursor,
                    &mut hist_pos,
                    &mut saved_line,
                    history,
                    &mut painter,
                    &mut out,
                )?;
            }
            _ => {
                for ch in read_char(&mut stdin, &mut byte)?.chars() {
                    chars.insert(cursor, ch);
                    cursor += 1;
                }
                painter.repaint(&mut out, &chars, cursor)?;
                last_was_tab = false;
            }
        }
    }
}

/// Reads the rest of an escape sequence (`ESC [ <key>` style) and applies the
/// supported keys: arrows (with history navigation), Home/End, Delete.
///
/// A bare `ESC` with nothing following (the Escape key on its own) reads 0
/// bytes and is correctly a no-op. `ESC` followed by anything other than
/// `[` is a meta/Alt chord (many terminals send `Alt+x` as `ESC x`): sibsh
/// has no bound action for those, but the byte itself is a real keystroke
/// and is inserted into the buffer verbatim instead of being dropped, so
/// Alt-chords degrade to "insert the plain key" rather than "lose a
/// keystroke with no feedback".
#[allow(clippy::too_many_arguments)]
fn handle_escape(
    stdin: &mut impl Read,
    byte: &mut [u8; 1],
    chars: &mut Vec<char>,
    cursor: &mut usize,
    hist_pos: &mut usize,
    saved_line: &mut Option<String>,
    history: &[String],
    painter: &mut Painter,
    out: &mut impl Write,
) -> io::Result<()> {
    if stdin.read(byte)? == 0 {
        return Ok(()); // bare Escape key: no-op.
    }
    if byte[0] != b'[' {
        // Not a CSI sequence: treat the byte as a literal keystroke rather
        // than silently discarding it.
        for ch in read_char(stdin, byte)?.chars() {
            chars.insert(*cursor, ch);
            *cursor += 1;
        }
        painter.repaint(out, chars, *cursor)?;
        return Ok(());
    }
    if stdin.read(byte)? == 0 {
        return Ok(());
    }

    // Move through history; `hist_pos == history.len()` means the live line.
    let mut browse = |up: bool| -> Option<Vec<char>> {
        let new_pos = if up {
            hist_pos.checked_sub(1)?
        } else if *hist_pos < history.len() {
            *hist_pos + 1
        } else {
            return None;
        };
        *hist_pos = new_pos;
        Some(
            if *hist_pos == history.len() {
                saved_line.take().unwrap_or_default()
            } else {
                // Leaving the live line saves it so Down can restore it.
                if up && *hist_pos + 1 == history.len() {
                    let current: String = chars.iter().collect();
                    saved_line.get_or_insert(current);
                }
                history[*hist_pos].clone()
            }
            .chars()
            .collect(),
        )
    };

    match byte[0] {
        b'A' | b'B' => {
            let up = byte[0] == b'A';
            if let Some(line) = browse(up) {
                *chars = line;
                *cursor = chars.len();
                painter.repaint(out, chars, *cursor)?;
            }
        }
        b'C' => {
            if *cursor < chars.len() {
                *cursor += 1;
                painter.repaint(out, chars, *cursor)?;
            }
        }
        b'D' => {
            if *cursor > 0 {
                *cursor -= 1;
                painter.repaint(out, chars, *cursor)?;
            }
        }
        b'H' => {
            *cursor = 0;
            painter.repaint(out, chars, *cursor)?;
        }
        b'F' => {
            *cursor = chars.len();
            painter.repaint(out, chars, *cursor)?;
        }
        b'3' => {
            // Delete key: sequence ends with `~`.
            let _ = stdin.read(byte)?;
            if *cursor < chars.len() {
                chars.remove(*cursor);
                painter.repaint(out, chars, *cursor)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Assembles one full UTF-8 character from the lead byte plus continuations.
fn read_char(stdin: &mut impl Read, byte: &mut [u8; 1]) -> io::Result<String> {
    let lead = byte[0];
    if lead < 0x20 {
        return Ok(String::new()); // ignore other control bytes
    }
    if lead < 0x80 {
        return Ok((lead as char).to_string());
    }

    let extra = if lead >= 0xF0 {
        3
    } else if lead >= 0xE0 {
        2
    } else {
        1
    };
    let mut bytes = vec![lead];
    for _ in 0..extra {
        if stdin.read(byte)? == 0 {
            break;
        }
        bytes.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aliases(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn completes_builtin_commands() {
        // Empty path list: only builtins can match, regardless of machine.
        let c = complete_in("ec", &[], &[]);
        assert_eq!(c.candidates, vec!["echo"]);
        assert_eq!(c.insert, "ho ");
    }

    #[test]
    fn completes_aliases() {
        let al = aliases(&[("gstat", "git status")]);
        let c = complete_in("gst", &al, &[]);
        assert_eq!(c.candidates, vec!["gstat"]);
        assert_eq!(c.insert, "at ");
    }

    #[test]
    fn empty_prefix_lists_all_builtins_and_path_executables() {
        // Synthetic PATH dir with one executable: deterministic on any machine.
        let dir = env::temp_dir().join("sibsh_fakepath_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        let exe = dir.join("fakebin");
        fs::write(&exe, "#!/bin/sh\n").expect("write");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).expect("chmod");
        }

        let dirs = vec![dir.clone()];
        let c = complete_in("", &[], &dirs);
        assert!(c.candidates.contains(&"echo".to_string()));
        assert!(c.candidates.contains(&"history".to_string()));
        assert!(c.candidates.contains(&"fakebin".to_string()));
        // Multiple candidates -> nothing unambiguous to insert.
        assert!(c.insert.is_empty());
        // Non-executable files in a PATH dir are ignored.
        fs::write(dir.join("notexec"), "x").expect("write");
        let c2 = complete_in("notexec", &[], &dirs);
        assert!(c2.candidates.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_candidates_gives_nothing() {
        let c = complete_in("zzzznope", &[], &[]);
        assert!(c.candidates.is_empty());
        assert!(c.insert.is_empty());
    }

    #[test]
    fn path_completion_in_temp_dir() {
        let dir = env::temp_dir().join("sibsh_completion_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).expect("mkdir");
        fs::write(dir.join("alpha.txt"), "a").expect("write");
        fs::write(dir.join("beta.txt"), "b").expect("write");
        fs::write(dir.join(".hidden"), "h").expect("write");

        let prefix = "cat ";
        let token = format!("{}/", dir.display());
        let line = format!("{prefix}{token}");

        let c = complete(&line, &[]);
        assert_eq!(
            c.candidates,
            vec![
                format!("{token}alpha.txt"),
                format!("{token}beta.txt"),
                format!("{token}sub/"),
            ]
        );
        // Hidden files are skipped unless requested.
        assert!(!c.candidates.iter().any(|p| p.contains("hidden")));

        // Directories get a trailing slash.
        let line = format!("cd {token}su");
        let c = complete(&line, &[]);
        assert_eq!(c.candidates, vec![format!("{token}sub/")]);
        assert_eq!(c.insert, "b/");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hidden_files_complete_when_requested() {
        let dir = env::temp_dir().join("sibsh_hidden_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        fs::write(dir.join(".sibshrc"), "x").expect("write");

        let line = format!("cat {}/.si", dir.display());
        let c = complete(&line, &[]);
        assert_eq!(c.candidates, vec![format!("{}/.sibshrc", dir.display())]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn second_word_completes_paths_not_commands() {
        // `echo ec` — the second word is a path lookup, not a command lookup.
        let c = complete("echo ec", &[]);
        assert!(c.candidates.is_empty(), "unexpected: {:?}", c.candidates);
    }

    #[test]
    fn common_prefix_handles_multibyte() {
        assert_eq!(common_prefix(&["ab".into(), "abc".into()]), "ab");
        assert_eq!(
            common_prefix(&["\u{e9}x".into(), "\u{e9}y".into()]),
            "\u{e9}"
        );
        assert_eq!(common_prefix(&[]), "");
    }

    #[test]
    fn common_prefix_single_candidate_is_itself() {
        assert_eq!(common_prefix(&["only".into()]), "only");
        assert_eq!(common_prefix(&["same".into(), "same".into()]), "same");
    }

    #[test]
    fn ambiguous_prefix_inserts_shared_part_only() {
        // Deterministic with an empty PATH list: only `history` matches.
        let c = complete_in("his", &[], &[]);
        assert_eq!(c.candidates, vec!["history"]);
        assert_eq!(c.insert, "tory ");
    }

    #[test]
    fn alias_and_builtin_deduplicated() {
        // Defining an alias named like an existing builtin must not duplicate it.
        let al = aliases(&[("echo", "echo -n")]);
        let c = complete_in("ec", &al, &[]);
        assert_eq!(c.candidates.iter().filter(|n| *n == "echo").count(), 1);
    }

    #[test]
    fn tab_separated_token_boundary_completes_as_second_word() {
        // A tab acts as a word separator, same as a space.
        let c = complete("echo\tzzzznope", &[]);
        assert!(c.candidates.is_empty());
        assert!(c.insert.is_empty());
    }

    #[test]
    fn display_width_skips_ansi_sequences() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("\x1b[38;5;242m╭─\x1b[0m x"), 4);
        assert_eq!(display_width("\x1b[1A\r\x1b[J"), 0);
        assert_eq!(display_width(""), 0);
    }

    #[test]
    fn csi_introducer_bracket_is_never_mistaken_for_a_final_byte() {
        // '[' (0x5B) sits inside the 0x40..=0x7E CSI final-byte range, but
        // it is the CSI *introducer*, not a final byte. A naive
        // "in_ansi && is_csi_final_byte(ch)" check applied to the byte
        // right after ESC would close the sequence one byte too early,
        // treating the rest of the sequence's parameter bytes and its
        // real final byte as visible text. `AnsiState` keeps "just saw
        // ESC" and "inside CSI params" as distinct states specifically to
        // prevent this.
        let mut state = AnsiState::Normal;
        assert!(!state.step('\x1b'));
        assert!(!state.step('['));
        assert_eq!(state, AnsiState::Csi, "'[' must enter Csi, not close it");
        assert!(!state.step('3'));
        assert!(!state.step('8'));
        assert!(!state.step('m'));
        assert_eq!(state, AnsiState::Normal, "'m' is the real final byte");
    }

    #[test]
    fn display_width_correctly_measures_real_sgr_sequences() {
        // The exact multi-parameter SGR shape `prompt.rs` emits, e.g.
        // `clr::BRAND` = "\x1b[1;38;5;141m". Regresses a bug where the
        // CSI introducer '[' was mistaken for a final byte, causing the
        // parameter digits and the true final byte to be measured as
        // visible width instead of skipped.
        let s = "\x1b[1;38;5;141m[sibsh]\x1b[0m \x1b[38;5;242m\u{256d}\u{2500}\x1b[0m";
        assert_eq!(display_width(s), 10); // "[sibsh] " + ╭─
    }

    #[test]
    fn multiple_consecutive_sgr_sequences_all_get_skipped() {
        let s = "\x1b[1m\x1b[38;5;9m\x1b[4mBOLD-RED-UNDERLINE\x1b[0m";
        assert_eq!(display_width(s), "BOLD-RED-UNDERLINE".chars().count());
    }

    #[test]
    fn display_width_closes_sequences_with_symbol_final_bytes() {
        // CSI final bytes are 0x40..=0x7E per ECMA-48, not only ASCII
        // letters; `@` is a legal (if unusual) final byte.
        let s = "\x1b[3@visible text";
        assert_eq!(display_width(s), "visible text".chars().count());
    }

    #[test]
    fn bare_non_csi_escape_is_swallowed_as_a_two_byte_sequence() {
        // ESC followed by anything other than '[' (e.g. stray ESC bytes
        // from pasted input) consumes exactly two bytes without
        // corrupting measurement of what follows.
        let s = "\x1bcvisible";
        assert_eq!(display_width(s), "visible".chars().count());
    }

    #[test]
    fn char_width_handles_multibyte_classes() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('é'), 1);
        assert_eq!(char_width('中'), 2);
        assert_eq!(char_width('│'), 1);
        // Nerd-font private-use glyphs occupy one cell.
        assert_eq!(char_width('\u{f418}'), 1);
        // Control characters take no cells.
        assert_eq!(char_width('\n'), 0);
    }

    #[test]
    fn render_rows_counts_wrapping() {
        let prompt = "top\n❯ ";
        // Short buffer: two logical lines -> two rows.
        assert_eq!(render_rows(prompt, "hi", 80), 2);
        // Buffer wraps the second line past 80 columns.
        assert_eq!(render_rows(prompt, &"z".repeat(200), 80), 4);
        // Exactly-full line does not spawn an extra row (deferred wrap).
        assert_eq!(render_rows("", &"z".repeat(80), 80), 1);
        assert_eq!(render_rows("", &"z".repeat(81), 80), 2);
        // Wide CJK characters count double toward wrapping.
        assert_eq!(render_rows("", &"中".repeat(41), 80), 2);
        // ANSI escapes add no width.
        assert_eq!(render_rows("\x1b[31m❯\x1b[0m ", &"z".repeat(78), 80), 1);
    }

    #[test]
    fn render_rows_still_correct_with_symbol_terminated_sequences() {
        let prompt = "\x1b[3@> ";
        assert_eq!(render_rows(prompt, "hello", 80), 1);
        assert_eq!(render_rows(prompt, &"x".repeat(78), 80), 1);
        assert_eq!(render_rows(prompt, &"x".repeat(79), 80), 2);
    }

    #[test]
    fn crlf_only_touches_newlines() {
        assert_eq!(crlf("a\nb"), "a\r\nb");
        assert_eq!(crlf("plain"), "plain");
        assert_eq!(crlf("\n"), "\r\n");
    }

    #[test]
    fn up_rows_covers_previous_render_and_candidates() {
        // Two-row render, nothing extra: move up one.
        assert_eq!(up_rows(2, 0, 40), 1);
        // Candidate listing adds its rows.
        assert_eq!(up_rows(2, 5, 40), 6);
        // Single-row render never moves up.
        assert_eq!(up_rows(1, 0, 40), 0);
        // Degenerate zero-row state stays at 0 (saturating).
        assert_eq!(up_rows(0, 0, 40), 0);
    }

    #[test]
    fn up_rows_never_walks_past_the_top_of_the_screen() {
        // A render taller than the screen has already scrolled; clamping to
        // screen_rows - 1 lands on row 0 instead of escaping into scrollback.
        assert_eq!(up_rows(50, 0, 24), 23);
        assert_eq!(up_rows(10, 20, 8), 7);
        // One-row screen can never move up.
        assert_eq!(up_rows(9, 9, 1), 0);
    }

    #[test]
    fn render_rows_remeasures_content_after_resize() {
        // The same content measured under a narrow width occupies more rows;
        // this is exactly what Painter recomputes when it detects a resize.
        let prompt = "top\n❯ ";
        let line = "z".repeat(60);
        let wide = render_rows(prompt, &line, 80);
        let narrow = render_rows(prompt, &line, 30);
        assert_eq!(wide, 2);
        assert!(narrow > wide);
    }
}
