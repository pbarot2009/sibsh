//! Interactive line reading: tab completion, basic line editing, and
//! history navigation. Uses `stty` for raw terminal mode so the project
//! stays dependency-free. When stdin is not a terminal (scripts, tests),
//! falls back to plain buffered line reading.

use crate::builtins::Builtins;
use crate::config::expand_tilde;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

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
    // `stty` fails when stdin is not a TTY (piped scripts, tests), which
    // selects the non-interactive fallback automatically.
    if stty(&["raw", "-echo"]) {
        let result = read_line_raw(prompt, history, aliases);
        let _ = stty(&["sane"]);
        result
    } else {
        read_line_plain(prompt)
    }
}

fn stty(args: &[&str]) -> bool {
    Command::new("stty")
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
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

fn redraw(out: &mut impl Write, prompt: &str, chars: &[char], cursor: usize) -> io::Result<()> {
    let line: String = chars.iter().collect();
    write!(out, "\r\x1b[K{prompt}{line}")?;
    if cursor < chars.len() {
        write!(out, "\x1b[{}D", chars.len() - cursor)?;
    }
    out.flush()
}

fn print_candidates(out: &mut impl Write, candidates: &[String]) -> io::Result<()> {
    const WIDTH: usize = 80;
    let longest = candidates.iter().map(String::len).max().unwrap_or(0);
    let columns = (WIDTH / (longest + 2)).max(1);

    for (idx, candidate) in candidates.iter().enumerate() {
        if idx % columns == 0 {
            writeln!(out)?;
        }
        write!(out, "{candidate:<longest$}  ")?;
    }
    writeln!(out)
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

    write!(out, "{prompt}")?;
    out.flush()?;

    loop {
        if stdin.read(&mut byte)? == 0 {
            writeln!(out)?;
            out.flush()?;
            return Ok(if chars.is_empty() {
                None
            } else {
                Some(chars.iter().collect())
            });
        }

        match byte[0] {
            b'\r' | b'\n' => {
                writeln!(out)?;
                out.flush()?;
                return Ok(Some(chars.iter().collect()));
            }
            CTRL_C => {
                writeln!(out, "^C")?;
                out.flush()?;
                return Ok(Some(String::new()));
            }
            CTRL_D => {
                if chars.is_empty() {
                    writeln!(out)?;
                    out.flush()?;
                    return Ok(None);
                }
                if cursor < chars.len() {
                    chars.remove(cursor);
                    redraw(&mut out, prompt, &chars, cursor)?;
                }
            }
            BACKSPACE | 0x08 => {
                if cursor > 0 {
                    cursor -= 1;
                    chars.remove(cursor);
                    redraw(&mut out, prompt, &chars, cursor)?;
                }
                last_was_tab = false;
            }
            b'\t' => {
                let prefix: String = chars[..cursor].iter().collect();
                let completion = complete(&prefix, aliases);

                if !completion.insert.is_empty() {
                    for ch in completion.insert.chars() {
                        chars.insert(cursor, ch);
                        cursor += 1;
                    }
                    redraw(&mut out, prompt, &chars, cursor)?;
                    last_was_tab = false;
                } else if last_was_tab && completion.candidates.len() > 1 {
                    print_candidates(&mut out, &completion.candidates)?;
                    redraw(&mut out, prompt, &chars, cursor)?;
                } else {
                    last_was_tab = true;
                }
            }
            CTRL_A => {
                cursor = 0;
                redraw(&mut out, prompt, &chars, cursor)?;
                last_was_tab = false;
            }
            CTRL_E => {
                cursor = chars.len();
                redraw(&mut out, prompt, &chars, cursor)?;
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
                    prompt,
                    &mut out,
                )?;
            }
            _ => {
                for ch in read_char(&mut stdin, &mut byte)?.chars() {
                    chars.insert(cursor, ch);
                    cursor += 1;
                }
                redraw(&mut out, prompt, &chars, cursor)?;
                last_was_tab = false;
            }
        }
    }
}

/// Reads the rest of an escape sequence (`ESC [ <key>` style) and applies the
/// supported keys: arrows (with history navigation), Home/End, Delete.
#[allow(clippy::too_many_arguments)]
fn handle_escape(
    stdin: &mut impl Read,
    byte: &mut [u8; 1],
    chars: &mut Vec<char>,
    cursor: &mut usize,
    hist_pos: &mut usize,
    saved_line: &mut Option<String>,
    history: &[String],
    prompt: &str,
    out: &mut impl Write,
) -> io::Result<()> {
    if stdin.read(byte)? == 0 || byte[0] != b'[' {
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
                redraw(out, prompt, chars, *cursor)?;
            }
        }
        b'C' => {
            if *cursor < chars.len() {
                *cursor += 1;
                redraw(out, prompt, chars, *cursor)?;
            }
        }
        b'D' => {
            if *cursor > 0 {
                *cursor -= 1;
                redraw(out, prompt, chars, *cursor)?;
            }
        }
        b'H' => {
            *cursor = 0;
            redraw(out, prompt, chars, *cursor)?;
        }
        b'F' => {
            *cursor = chars.len();
            redraw(out, prompt, chars, *cursor)?;
        }
        b'3' => {
            // Delete key: sequence ends with `~`.
            let _ = stdin.read(byte)?;
            if *cursor < chars.len() {
                chars.remove(*cursor);
                redraw(out, prompt, chars, *cursor)?;
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
}
