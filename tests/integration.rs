//! Integration tests: drive the real `sibsh` binary with stdin scripts and
//! verify file side effects. Covers the Phase 1.2 edge-case matrix from
//! CHECKLISTS.md. No external dependencies — std only.

use std::fs;
use std::process::{Command, Stdio};

fn run_shell(script: &str) -> (String, i32) {
    use std::io::Write;

    let mut child = Command::new(env!("CARGO_BIN_EXE_sibsh"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn sibsh");

    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("failed to write script");

    let output = child.wait_with_output().expect("failed to wait");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

/// Strips ANSI color codes and the prompt prefix from captured output.
fn visible_lines(output: &str) -> Vec<String> {
    let mut plain = String::new();
    let mut in_ansi = false;
    for ch in output.chars() {
        match ch {
            '\x1b' => in_ansi = true,
            'm' if in_ansi => in_ansi = false,
            _ if !in_ansi => plain.push(ch),
            _ => {}
        }
    }
    plain
        .lines()
        .map(|line| match line.rsplit_once('❯') {
            // Keep only what follows the prompt marker; continuation lines
            // of multi-line output have no marker and are kept verbatim.
            Some((_, rest)) => rest.trim(),
            None => line,
        })
        .filter(|rest| !rest.is_empty())
        .map(str::to_string)
        .collect()
}

/// Strips only ANSI color codes, keeping prompt markers like `[1]`.
fn plain_text(output: &str) -> String {
    let mut plain = String::new();
    let mut in_ansi = false;
    for ch in output.chars() {
        match ch {
            '\x1b' => in_ansi = true,
            'm' if in_ansi => in_ansi = false,
            _ if !in_ansi => plain.push(ch),
            _ => {}
        }
    }
    plain
}

fn tmpfile(name: &str) -> String {
    format!("{}/sibsh_it_{}", std::env::temp_dir().display(), name)
}

macro_rules! cleanup {
    ($($file:expr),+ $(,)?) => {{
        $(let _ = fs::remove_file($file);)+
    }};
}

#[test]
fn matrix_1_output_trunc_writes_file_not_terminal() {
    let f = tmpfile("m1.txt");
    let (out, code) = run_shell(&format!("echo hello > {f}\n"));
    assert_eq!(code, 0);
    assert!(!visible_lines(&out).iter().any(|l| l.contains("hello")));
    assert_eq!(fs::read_to_string(&f).unwrap(), "hello\n");
    cleanup!(f);
}

#[test]
fn matrix_2_append_keeps_existing_content() {
    let f = tmpfile("m2.txt");
    run_shell(&format!("echo a > {f}\necho b >> {f}\n"));
    assert_eq!(fs::read_to_string(&f).unwrap(), "a\nb\n");
    cleanup!(f);
}

#[test]
fn matrix_3_trunc_replaces_content() {
    let f = tmpfile("m3.txt");
    run_shell(&format!("echo xxxxxxxxxxxx > {f}\necho y > {f}\n"));
    assert_eq!(fs::read_to_string(&f).unwrap(), "y\n");
    cleanup!(f);
}

#[test]
fn matrix_4_input_redirect_feeds_cat() {
    let f = tmpfile("m4.txt");
    fs::write(&f, "line1\nline2\n").unwrap();
    let (out, code) = run_shell(&format!("cat < {f}\n"));
    assert_eq!(code, 0);
    let lines = visible_lines(&out);
    assert!(lines.contains(&"line1".to_string()));
    assert!(lines.contains(&"line2".to_string()));
    cleanup!(f);
}

#[test]
fn matrix_5_external_cmd_both_directions() {
    let input = tmpfile("m5_in.txt");
    let output = tmpfile("m5_out.txt");
    fs::write(&input, "pear\napple\n").unwrap();
    let (out, code) = run_shell(&format!("sort < {input} > {output}\n"));
    assert_eq!(code, 0);
    assert_eq!(fs::read_to_string(&output).unwrap(), "apple\npear\n");
    assert!(!visible_lines(&out).iter().any(|l| l.contains("apple")));
    cleanup!(input, output);
}

#[test]
fn matrix_6_quoted_operator_is_literal() {
    let (out, code) = run_shell("echo 'a > b'\n");
    assert_eq!(code, 0);
    assert!(visible_lines(&out).contains(&"a > b".to_string()));
}

#[test]
fn matrix_7_dquoted_operator_is_literal() {
    let (out, code) = run_shell("echo \">\"\n");
    assert_eq!(code, 0);
    assert!(visible_lines(&out).contains(&">".to_string()));
}

#[test]
fn matrix_8_attached_form_no_space() {
    let f = tmpfile("m8.txt");
    let (out, code) = run_shell(&format!("echo hi >{f}\n"));
    assert_eq!(code, 0);
    assert_eq!(fs::read_to_string(&f).unwrap(), "hi\n");
    assert!(!visible_lines(&out).iter().any(|l| l.contains("hi")));
    cleanup!(f);
}

#[test]
fn matrix_9_missing_input_file_errors_and_skips_command() {
    let f = tmpfile("m9_missing.txt");
    let (out, code) = run_shell(&format!("cat < {f}\ntrue\n"));
    assert_eq!(code, 0);
    // The failure must be visible in the next prompt's status indicator.
    assert!(plain_text(&out).contains("[1]"), "expected [1] in: {out}");
}

#[test]
fn matrix_10_unwritable_target_errors() {
    let (out, code) = run_shell("echo hi > /nonexistent_dir_xyz/f.txt\ntrue\n");
    assert_eq!(code, 0);
    assert!(plain_text(&out).contains("[1]"), "expected [1] in: {out}");
}

#[test]
fn matrix_11_missing_filename_is_syntax_error() {
    let (out, code) = run_shell("echo hi >\ntrue\n");
    assert_eq!(code, 0);
    assert!(plain_text(&out).contains("[1]"), "expected [1] in: {out}");
}

#[test]
fn matrix_12_expansion_in_filename_and_content() {
    let f = tmpfile("m12.txt");
    let (out, code) = run_shell(&format!(
        "export SIBSH_IT=hello\necho $SIBSH_IT > {f}\ncat < {f}\n"
    ));
    assert_eq!(code, 0);
    assert!(visible_lines(&out).contains(&"hello".to_string()));
    assert_eq!(fs::read_to_string(&f).unwrap(), "hello\n");
    cleanup!(f);
}

#[test]
fn matrix_13_failing_cmd_still_creates_empty_log() {
    let f = tmpfile("m13.log");
    let (_, code) = run_shell(&format!("false > {f}\n"));
    assert_eq!(code, 1);
    assert_eq!(fs::read_to_string(&f).unwrap(), "");
    cleanup!(f);
}

#[test]
fn matrix_14_builtin_copy_via_two_redirects() {
    let src = tmpfile("m14_src.txt");
    let dst = tmpfile("m14_dst.txt");
    fs::write(&src, "copy me\n").unwrap();
    let (_, code) = run_shell(&format!("cat < {src} > {dst}\n"));
    assert_eq!(code, 0);
    assert_eq!(fs::read_to_string(&dst).unwrap(), "copy me\n");
    cleanup!(src, dst);
}

#[test]
fn matrix_15_external_binary_reads_redirected_stdin() {
    let f = tmpfile("m15.txt");
    fs::write(&f, "one\ntwo\nthree\n").unwrap();
    let (out, code) = run_shell(&format!("wc -l < {f}\n"));
    assert_eq!(code, 0);
    assert!(visible_lines(&out).iter().any(|l| l.starts_with('3')));
    cleanup!(f);
}

#[test]
fn regression_builtin_stdout_redirect() {
    let f = tmpfile("reg.txt");
    let (out, code) = run_shell(&format!("history > {f}\n"));
    assert_eq!(code, 0);
    let contents = fs::read_to_string(&f).unwrap();
    assert!(contents.contains("history"), "history line must be in file");
    assert!(!visible_lines(&out).iter().any(|l| l.contains("history")));
    cleanup!(f);
}

#[test]
fn regression_exit_code_still_propagates() {
    let (_, code) = run_shell("exit 7\n");
    assert_eq!(code, 7);
}

#[test]
fn regression_phase_1_1_behavior_intact() {
    let (out, code) = run_shell("echo plain\n");
    assert_eq!(code, 0);
    assert!(visible_lines(&out).contains(&"plain".to_string()));
}
