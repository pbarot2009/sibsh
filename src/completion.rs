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
pub fn read_line(
    prompt: &str,
    history: &[String],
    aliases: &[(String, String)],
) -> io::Result<Option<String>> {
    match tty::with_raw_mode(|| read_line_raw(prompt, history, aliases)) {
        Some(result) => result,
        None => read_line_plain(prompt),
    }
}

fn read_line_plain(prompt: &str) -> io::Result<Option<String>> {
    print!("{prompt}");
    let _ = io::stdout().flush();

    let mut line = String::new();
    match io::stdin().read_line(&mut line)? {
        0 => Ok(None),
        _ => Ok(Some(line)),
    }
}

/// One step of ANSI-escape-sequence recognition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnsiState {
    Normal,
    Escaped,
    Csi,
}

impl AnsiState {
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

fn is_csi_final_byte(ch: char) -> bool {
    matches!(ch as u32, 0x40..=0x7e)
}

/// Visible terminal cell width of `s`, skipping ANSI escape sequences.
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
        0x00..=0x1f | 0x7f | 0x0300..=0x036f | 0x200b..=0x200f | 0xfe00..=0xfe0f => 0,
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

/// Rows the repaint must move the cursor up.
fn up_rows(base_rows: usize, extra_rows: usize, screen_rows: usize) -> usize {
    (base_rows.saturating_sub(1) + extra_rows).min(screen_rows.saturating_sub(1))
}

/// Number of screen rows the render occupies when drawn from column 0.
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
            if term_cols > 0 && col + cw > term_cols {
                rows += 1;
                col = cw;
            } else {
                col += cw;
            }
        }
    }
    if col == term_cols && term_cols > 0 {
        rows += 1;
    }
    rows
}

/// Computes the 0-indexed (row, col) coordinates of the end of `chars`
/// rendered after `prompt` in a `term_cols`-wide terminal.
fn content_geometry(prompt: &str, chars: &[char], term_cols: usize) -> (usize, usize) {
    let mut row = 0usize;
    let mut col = 0usize;
    let mut state = AnsiState::Normal;
    for ch in prompt.chars() {
        if !state.step(ch) {
            continue;
        }
        if ch == '\n' {
            row += 1;
            col = 0;
        } else {
            let cw = char_width(ch);
            if term_cols > 0 && col + cw > term_cols {
                row += 1;
                col = cw;
            } else {
                col += cw;
            }
        }
    }
    for &ch in chars {
        let cw = char_width(ch);
        if term_cols > 0 && col + cw > term_cols {
            row += 1;
            col = cw;
        } else {
            col += cw;
        }
    }
    (row, col)
}

/// Owns everything needed to repaint the editing area.
struct Painter {
    /// The final line of the prompt (the edit line, e.g. `╰─ > `).
    prompt_last_line: String,
    term_cols: usize,
    /// Screen rows occupied by the active edit area (`prompt_last_line` + `line`).
    base_rows: usize,
    /// Rows appended *below* the edit area since the last full repaint.
    extra_rows: usize,
    /// The line content as of the last paint.
    last_line: String,
}

impl Painter {
    fn new(prompt: &str, out: &mut impl Write) -> io::Result<Self> {
        let term_cols = tty::terminal_width().max(1);
        let prompt_last_line = match prompt.rfind('\n') {
            Some(idx) => prompt[idx + 1..].to_string(),
            None => prompt.to_string(),
        };
        // Print the full initial prompt once.
        write!(out, "{}", crlf(prompt))?;
        out.flush()?;
        Ok(Self {
            prompt_last_line: prompt_last_line.clone(),
            term_cols,
            base_rows: render_rows(&prompt_last_line, "", term_cols),
            extra_rows: 0,
            last_line: String::new(),
        })
    }

    fn repaint(&mut self, out: &mut impl Write, chars: &[char], cursor: usize) -> io::Result<()> {
        let line: String = chars.iter().collect();
        let (screen_rows, raw_cols) = tty::terminal_size();
        let cols = raw_cols.max(1);
        if cols != self.term_cols {
            self.term_cols = cols;
            self.base_rows = render_rows(&self.prompt_last_line, &self.last_line, cols);
        }
        let up = up_rows(self.base_rows, self.extra_rows, screen_rows);
        if up > 0 {
            write!(out, "\x1b[{up}A")?;
        }
        // Repaint only the active edit line and buffer.
        write!(out, "\r\x1b[J{}{line}", crlf(&self.prompt_last_line))?;

        // Position the cursor accurately across wrapped rows.
        let (end_row, _) = content_geometry(&self.prompt_last_line, chars, self.term_cols);
        let (cur_row, cur_col) =
            content_geometry(&self.prompt_last_line, &chars[..cursor], self.term_cols);

        if end_row > cur_row {
            let row_diff = end_row - cur_row;
            write!(out, "\x1b[{row_diff}A\r")?;
            if cur_col > 0 {
                write!(out, "\x1b[{cur_col}C")?;
            }
        } else {
            let behind: usize = chars[cursor..].iter().map(|c| char_width(*c)).sum();
            if behind > 0 {
                write!(out, "\x1b[{behind}D")?;
            }
        }

        self.base_rows = render_rows(&self.prompt_last_line, &line, self.term_cols);
        self.extra_rows = 0;
        self.last_line = line;
        out.flush()
    }
}

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
        let pad = longest - display_width(candidate);
        write!(out, "{}  ", " ".repeat(pad))?;
    }
    write!(out, "\r\n")?;
    Ok(candidates.len().div_ceil(columns) + 1)
}

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
        painter.repaint(out, chars, chars.len())?;
        let rows = print_candidates(out, &completion.candidates)?;
        painter.extra_rows += rows;
        painter.repaint(out, chars, *cursor)?;
    } else {
        *last_was_tab = true;
    }
    Ok(())
}

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
                painter.repaint(&mut out, &chars, chars.len())?;
                write!(out, "\r\n")?;
                out.flush()?;
                return Ok(Some(chars.iter().collect()));
            }
            CTRL_C => {
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
        return Ok(());
    }
    if byte[0] != b'[' {
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

fn read_char(stdin: &mut impl Read, byte: &mut [u8; 1]) -> io::Result<String> {
    let lead = byte[0];
    if lead < 0x20 {
        return Ok(String::new());
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
        assert!(c.insert.is_empty());
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
        assert!(!c.candidates.iter().any(|p| p.contains("hidden")));

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
        let c = complete_in("his", &[], &[]);
        assert_eq!(c.candidates, vec!["history"]);
        assert_eq!(c.insert, "tory ");
    }

    #[test]
    fn alias_and_builtin_deduplicated() {
        let al = aliases(&[("echo", "echo -n")]);
        let c = complete_in("ec", &al, &[]);
        assert_eq!(c.candidates.iter().filter(|n| *n == "echo").count(), 1);
    }

    #[test]
    fn tab_separated_token_boundary_completes_as_second_word() {
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
        let mut state = AnsiState::Normal;
        assert!(!state.step('\x1b'));
        assert!(!state.step('['));
        assert_eq!(state, AnsiState::Csi);
        assert!(!state.step('3'));
        assert!(!state.step('8'));
        assert!(!state.step('m'));
        assert_eq!(state, AnsiState::Normal);
    }

    #[test]
    fn display_width_correctly_measures_real_sgr_sequences() {
        let s = "\x1b[1;38;5;141m[sibsh]\x1b[0m \x1b[38;5;242m\u{256d}\u{2500}\x1b[0m";
        assert_eq!(display_width(s), 10);
    }

    #[test]
    fn multiple_consecutive_sgr_sequences_all_get_skipped() {
        let s = "\x1b[1m\x1b[38;5;9m\x1b[4mBOLD-RED-UNDERLINE\x1b[0m";
        assert_eq!(display_width(s), "BOLD-RED-UNDERLINE".chars().count());
    }

    #[test]
    fn display_width_closes_sequences_with_symbol_final_bytes() {
        let s = "\x1b[3@visible text";
        assert_eq!(display_width(s), "visible text".chars().count());
    }

    #[test]
    fn bare_non_csi_escape_is_swallowed_as_a_two_byte_sequence() {
        let s = "\x1bcvisible";
        assert_eq!(display_width(s), "visible".chars().count());
    }

    #[test]
    fn char_width_handles_multibyte_classes() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('é'), 1);
        assert_eq!(char_width('中'), 2);
        assert_eq!(char_width('│'), 1);
        assert_eq!(char_width('\u{f418}'), 1);
        assert_eq!(char_width('\n'), 0);
    }

    #[test]
    fn render_rows_counts_wrapping() {
        let prompt = "top\n❯ ";
        assert_eq!(render_rows(prompt, "hi", 80), 2);
        assert_eq!(render_rows(prompt, &"z".repeat(200), 80), 4);
        assert_eq!(render_rows("", &"z".repeat(80), 80), 2);
        assert_eq!(render_rows("", &"z".repeat(81), 80), 2);
        assert_eq!(render_rows("", &"中".repeat(41), 80), 2);
        assert_eq!(render_rows("\x1b[31m❯\x1b[0m ", &"z".repeat(78), 80), 2);
    }

    #[test]
    fn render_rows_still_correct_with_symbol_terminated_sequences() {
        let prompt = "\x1b[3@> ";
        assert_eq!(render_rows(prompt, "hello", 80), 1);
        assert_eq!(render_rows(prompt, &"x".repeat(78), 80), 2);
        assert_eq!(render_rows(prompt, &"x".repeat(79), 80), 2);
    }

    #[test]
    fn content_geometry_tracks_rows_and_cols() {
        let prompt = "❯ ";
        let chars: Vec<char> = "hello".chars().collect();
        let (row, col) = content_geometry(prompt, &chars, 80);
        assert_eq!(row, 0);
        assert_eq!(col, 7);

        let chars_wrapped: Vec<char> = "a".repeat(10).chars().collect();
        let (row, col) = content_geometry(prompt, &chars_wrapped, 6);
        assert_eq!(row, 2);
        assert_eq!(col, 2);
    }

    #[test]
    fn crlf_only_touches_newlines() {
        assert_eq!(crlf("a\nb"), "a\r\nb");
        assert_eq!(crlf("plain"), "plain");
        assert_eq!(crlf("\n"), "\r\n");
    }

    #[test]
    fn up_rows_covers_previous_render_and_candidates() {
        assert_eq!(up_rows(2, 0, 40), 1);
        assert_eq!(up_rows(2, 5, 40), 6);
        assert_eq!(up_rows(1, 0, 40), 0);
        assert_eq!(up_rows(0, 0, 40), 0);
    }

    #[test]
    fn up_rows_never_walks_past_the_top_of_the_screen() {
        assert_eq!(up_rows(50, 0, 24), 23);
        assert_eq!(up_rows(10, 20, 8), 7);
        assert_eq!(up_rows(9, 9, 1), 0);
    }

    #[test]
    fn render_rows_remeasures_content_after_resize() {
        let prompt = "top\n❯ ";
        let line = "z".repeat(60);
        let wide = render_rows(prompt, &line, 80);
        let narrow = render_rows(prompt, &line, 30);
        assert_eq!(wide, 2);
        assert!(narrow > wide);
    }
}
