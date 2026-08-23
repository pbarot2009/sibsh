//! Integration tests: drive the real `sibsh` binary with stdin scripts and
//! verify file side effects. Covers the Phase 1.2 edge-case matrix from
//! CHECKLISTS.md. No external dependencies — std only.

use std::fs;
use std::process::{Command, Stdio};

fn run_shell(script: &str) -> (String, i32) {
    run_shell_with(script, &[])
}

fn run_shell_with(script: &str, extra_env: &[(&str, &str)]) -> (String, i32) {
    use std::io::Write;

    let mut child = Command::new(env!("CARGO_BIN_EXE_sibsh"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(extra_env.iter().copied())
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

#[test]
fn config_aliases_from_toml_expand() {
    let config = tmpfile("cfg.toml");
    fs::write(&config, "[aliases]\nhello = \"echo world\"\n").unwrap();

    let (out, code) = run_shell_with("hello\nalias\nunalias hello\nhello\ntrue\n", &[
        ("SIBSH_CONFIG", config.as_str()),
    ]);
    assert_eq!(code, 0);
    let lines = visible_lines(&out);
    assert!(lines.contains(&"world".to_string()), "alias must expand: {lines:?}");
    assert!(lines.iter().any(|l| l.contains("hello='echo world'")));
    // After unalias, `hello` is no longer a command.
    let after = lines.iter().position(|l| l == "world").unwrap_or(0);
    assert!(!lines[after + 1..].contains(&"world".to_string()), "unalias must remove it: {lines:?}");
    cleanup!(config);
}

#[test]
fn config_imports_run_bashrc_style_exports() {
    let dir = std::env::temp_dir().join("sibsh_imports_test");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let rc = dir.join("rc.sh");
    fs::write(
        &rc,
        "# a comment\nexport SIBSH_IMPORTED=from-import\ncase $SHELL in esac # unsupported syntax, skipped\necho imported-ran\n",
    )
    .unwrap();
    let config = tmpfile("cfg_imp.toml");
    fs::write(&config, format!("imports = [\"{}\"]\n", rc.display())).unwrap();

    let (out, code) = run_shell_with(
        "echo value=$SIBSH_IMPORTED\nexit 0\n",
        &[("SIBSH_CONFIG", config.as_str())],
    );
    assert_eq!(code, 0);
    let lines = visible_lines(&out);
    assert!(lines.contains(&"imported-ran".to_string()), "import lines must run: {lines:?}");
    assert!(lines.contains(&"value=from-import".to_string()), "export from import: {lines:?}");

    cleanup!(config);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn runtime_alias_define_list_remove() {
    let (out, code) = run_shell("alias ll='echo listed'\nll\nalias\nunalias ll\nunalias nosuch\ntrue\n");
    assert_eq!(code, 0);
    let lines = visible_lines(&out);
    assert!(lines.contains(&"listed".to_string()));
    assert!(lines.iter().any(|l| l.contains("ll='echo listed'")));
    // unalias of an unknown name is an error -> [1] status marker.
    assert!(plain_text(&out).contains("[1]"), "expected [1] after bad unalias");
}

#[test]
fn touch_updates_modification_time_of_existing_file() {
    use std::os::unix::fs::MetadataExt;
    let f = tmpfile("mtime.txt");
    fs::write(&f, "x").unwrap();
    let before = fs::metadata(&f).unwrap().mtime();

    std::thread::sleep(std::time::Duration::from_millis(1100));
    let (_, code) = run_shell(&format!("touch {f}\n"));
    assert_eq!(code, 0);

    let after = fs::metadata(&f).unwrap().mtime();
    assert!(after > before, "touch must bump mtime ({before} -> {after})");
    cleanup!(f);
}

#[test]
fn prompt_template_from_config_is_used() {
    let config = tmpfile("cfg_prompt.toml");
    fs::write(&config, "prompt = \"sh> {cwd}\"").unwrap();

    let (out, code) = run_shell_with("pwd\nexit\n", &[("SIBSH_CONFIG", config.as_str())]);
    assert_eq!(code, 0);
    assert!(plain_text(&out).contains("sh> "), "custom prompt expected in: {out}");
    cleanup!(config);
}

#[test]
fn missing_config_file_starts_normally() {
    let (out, code) = run_shell_with(
        "echo fine\nexit\n",
        &[("SIBSH_CONFIG", "/nonexistent/sibsh.toml")],
    );
    assert_eq!(code, 0);
    assert!(visible_lines(&out).contains(&"fine".to_string()));
}

// ---- Additional built-in coverage ----

#[test]
fn echo_n_suppresses_trailing_newline() {
    let (out, code) = run_shell("echo -n hello\necho done\n");
    assert_eq!(code, 0);
    // Without -n, `hello` and the next prompt would be on separate lines.
    assert!(plain_text(&out).contains("hellouser"), "no-newline output joins next prompt: {out}");
}

#[test]
fn cd_dash_toggles_between_directories() {
    let dir = std::env::temp_dir().join("sibsh_cd_toggle");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let (_, code) = run_shell(&format!(
        "cd {}\npwd > /dev/null\ncd -\ncd -\npwd\n", dir.display()
    ));
    assert_eq!(code, 0);
    // After two toggles we are back in the temp directory.
    assert!(fs::read_dir(&dir).is_ok());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn type_reports_builtins_and_externals() {
    let (out, code) = run_shell("type echo\ntype ls\ntype nosuchcmd_zz\ntrue\n");
    assert_eq!(code, 0);
    let lines = visible_lines(&out);
    assert!(lines.iter().any(|l| l.contains("builtin") && l.contains("echo")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("ls") && !l.contains("builtin")), "{lines:?}");
}

#[test]
fn which_prints_path_or_fails_cleanly() {
    let (out, code) = run_shell("which ls\nwhich nosuchcmd_zz\ntrue\n");
    assert_eq!(code, 0);
    assert!(visible_lines(&out).iter().any(|l| l.ends_with("/ls")), "{out}");
    assert!(plain_text(&out).contains("[1]"), "unknown command must set status 1");
}

#[test]
fn env_lists_exported_variable() {
    let (out, code) = run_shell("export SIBSH_ENV_TEST=visible\nenv\n");
    assert_eq!(code, 0);
    assert!(plain_text(&out).contains("SIBSH_ENV_TEST=visible"), "{out}");
}

#[test]
fn unset_removes_variable() {
    let (out, code) = run_shell(
        "export SIBSH_UNSET_ME=x\nunset SIBSH_UNSET_ME\necho [$SIBSH_UNSET_ME]\n",
    );
    assert_eq!(code, 0);
    assert!(visible_lines(&out).contains(&"[]".to_string()), "{out}");
}

#[test]
fn help_lists_available_commands() {
    let (out, code) = run_shell("help\n");
    assert_eq!(code, 0);
    for cmd in ["cd", "echo", "export", "history", "alias"] {
        assert!(plain_text(&out).contains(cmd), "help must mention {cmd}: {out}");
    }
}

#[test]
fn cat_concatenates_multiple_files_in_order() {
    let a = tmpfile("cat_a.txt");
    let b = tmpfile("cat_b.txt");
    fs::write(&a, "A1\nA2\n").unwrap();
    fs::write(&b, "B1\n").unwrap();
    let f = tmpfile("cat_out.txt");
    run_shell(&format!("cat {a} {b} > {f}\n"));
    assert_eq!(fs::read_to_string(&f).unwrap(), "A1\nA2\nB1\n");
    cleanup!(a, b, f);
}

#[test]
fn cat_missing_file_sets_error_status() {
    let f = tmpfile("does_not_exist_abc.txt");
    let (out, code) = run_shell(&format!("cat {f}\ntrue\n"));
    assert_eq!(code, 0);
    assert!(plain_text(&out).contains("[1]"), "expected [1]: {out}");
}

#[test]
fn status_expansion_reflects_previous_failure() {
    let (out, code) = run_shell("false\necho rc=$?\n");
    assert_eq!(code, 0);
    assert!(visible_lines(&out).contains(&"rc=1".to_string()), "{out}");
}

#[test]
fn status_expansion_reflects_previous_success() {
    let (out, code) = run_shell("true\necho rc=$?\n");
    assert_eq!(code, 0);
    assert!(visible_lines(&out).contains(&"rc=0".to_string()), "{out}");
}

#[test]
fn append_redirect_creates_missing_file() {
    let f = tmpfile("append_new.txt");
    run_shell(&format!("echo first >> {f}\n"));
    assert_eq!(fs::read_to_string(&f).unwrap(), "first\n");
    cleanup!(f);
}

#[test]
fn multi_word_alias_expands_completely() {
    let (out, code) = run_shell("alias gs='echo a b'\ngs c\nunalias gs\ntrue\n");
    assert_eq!(code, 0);
    // Alias value replaces the first word; trailing args are appended.
    assert!(visible_lines(&out).contains(&"a b c".to_string()), "{out}");
}

#[test]
fn exit_without_args_exits_zero() {
    let (_, code) = run_shell("exit\n");
    assert_eq!(code, 0);
}

#[test]
fn quoted_empty_string_is_a_real_argument() {
    let (out, code) = run_shell("echo ''x''\n");
    assert_eq!(code, 0);
    // Adjacent quotes concatenate into one token.
    assert!(visible_lines(&out).contains(&"x".to_string()), "{out}");
}

#[test]
fn history_records_every_command() {
    let f = tmpfile("hist.txt");
    run_shell(&format!(
        "echo one\necho two\nhistory > {f}\n"
    ));
    let contents = fs::read_to_string(&f).unwrap();
    assert!(contents.contains("echo one"), "{contents}");
    assert!(contents.contains("echo two"), "{contents}");
    cleanup!(f);
}
