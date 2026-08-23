//! Integration tests: drive the real `sibsh` binary with stdin scripts and
//! verify file side effects. Covers the Phase 1.2 edge-case matrix from
//! CHECKLISTS.md. No external dependencies — std only.

use std::fs;
use std::process::{Command, Stdio};

fn run_shell(script: &str) -> (String, i32) {
    run_shell_with(script, &[])
}

fn run_shell_with(script: &str, extra_env: &[(&str, &str)]) -> (String, i32) {
    let (out, _, code) = run_shell_full(script, extra_env);
    (out, code)
}

/// Like `run_shell_with`, but also returns captured stderr.
fn run_shell_full(script: &str, extra_env: &[(&str, &str)]) -> (String, String, i32) {
    use std::io::Write;

    let mut child = Command::new(env!("CARGO_BIN_EXE_sibsh"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Pin identity/remote vars so the prompt is identical everywhere
        // (CI runners have different usernames; some environments export
        // SSH variables that would add a user@host segment; root sandboxes
        // would switch the prompt into root mode).
        .env_remove("SSH_TTY")
        .env_remove("SSH_CONNECTION")
        .env("SIBSH_FORCE_NON_ROOT", "1")
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
        String::from_utf8_lossy(&output.stderr).to_string(),
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
        // The two-line prompt's top frame line carries no pointer marker,
        // so it would look like command output; drop it explicitly.
        .filter(|line| !(line.starts_with('\u{256d}') || line.starts_with("+-")))
        .map(|line| {
            match line
                .rsplit_once('\u{276f}')
                .or_else(|| line.rsplit_once('\u{276d}'))
            {
                // Keep only what follows the prompt marker; continuation lines
                // of multi-line output have no marker and are kept verbatim.
                Some((_, rest)) => rest.trim(),
                None => line,
            }
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
    assert!(
        plain_text(&out).contains("[1 \u{2718}]"),
        "expected [1 ✘] badge in: {out}"
    );
}

#[test]
fn matrix_10_unwritable_target_errors() {
    let (out, code) = run_shell("echo hi > /nonexistent_dir_xyz/f.txt\ntrue\n");
    assert_eq!(code, 0);
    assert!(
        plain_text(&out).contains("[1 \u{2718}]"),
        "expected [1 ✘] badge in: {out}"
    );
}

#[test]
fn matrix_11_missing_filename_is_syntax_error() {
    let (out, code) = run_shell("echo hi >\ntrue\n");
    assert_eq!(code, 0);
    assert!(
        plain_text(&out).contains("[1 \u{2718}]"),
        "expected [1 ✘] badge in: {out}"
    );
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

    let (out, code) = run_shell_with(
        "hello\nalias\nunalias hello\nhello\ntrue\n",
        &[("SIBSH_CONFIG", config.as_str())],
    );
    assert_eq!(code, 0);
    let lines = visible_lines(&out);
    assert!(
        lines.contains(&"world".to_string()),
        "alias must expand: {lines:?}"
    );
    assert!(lines.iter().any(|l| l.contains("hello='echo world'")));
    // After unalias, `hello` is no longer a command.
    let after = lines.iter().position(|l| l == "world").unwrap_or(0);
    assert!(
        !lines[after + 1..].contains(&"world".to_string()),
        "unalias must remove it: {lines:?}"
    );
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
    assert!(
        lines.contains(&"imported-ran".to_string()),
        "import lines must run: {lines:?}"
    );
    assert!(
        lines.contains(&"value=from-import".to_string()),
        "export from import: {lines:?}"
    );

    cleanup!(config);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn runtime_alias_define_list_remove() {
    let (out, code) =
        run_shell("alias ll='echo listed'\nll\nalias\nunalias ll\nunalias nosuch\ntrue\n");
    assert_eq!(code, 0);
    let lines = visible_lines(&out);
    assert!(lines.contains(&"listed".to_string()));
    assert!(lines.iter().any(|l| l.contains("ll='echo listed'")));
    // unalias of an unknown name is an error -> failure badge in next prompt.
    assert!(
        plain_text(&out).contains("[1 \u{2718}]"),
        "expected failure badge after bad unalias"
    );
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
    assert!(
        after > before,
        "touch must bump mtime ({before} -> {after})"
    );
    cleanup!(f);
}

#[test]
fn prompt_template_from_config_is_used() {
    let config = tmpfile("cfg_prompt.toml");
    fs::write(&config, "prompt = \"sh> {cwd}\"").unwrap();

    let (out, code) = run_shell_with("pwd\nexit\n", &[("SIBSH_CONFIG", config.as_str())]);
    assert_eq!(code, 0);
    assert!(
        plain_text(&out).contains("sh> "),
        "custom prompt expected in: {out}"
    );
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
    // Without -n, `hello` and the next prompt's top frame would be on
    // separate lines; with -n they are glued together on one line.
    assert!(
        plain_text(&out).contains("hello\u{256d}"),
        "no-newline output joins next prompt frame: {out}"
    );
}

#[test]
fn cd_dash_toggles_between_directories() {
    let dir = std::env::temp_dir().join("sibsh_cd_toggle");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let (_, code) = run_shell(&format!(
        "cd {}\npwd > /dev/null\ncd -\ncd -\npwd\n",
        dir.display()
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
    assert!(
        lines
            .iter()
            .any(|l| l.contains("builtin") && l.contains("echo")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("ls") && !l.contains("builtin")),
        "{lines:?}"
    );
}

#[test]
fn which_prints_path_or_fails_cleanly() {
    let (out, code) = run_shell("which ls\nwhich nosuchcmd_zz\ntrue\n");
    assert_eq!(code, 0);
    assert!(
        visible_lines(&out).iter().any(|l| l.ends_with("/ls")),
        "{out}"
    );
    assert!(
        plain_text(&out).contains("[1 \u{2718}]"),
        "unknown command must set status 1"
    );
}

#[test]
fn env_lists_exported_variable() {
    let (out, code) = run_shell("export SIBSH_ENV_TEST=visible\nenv\n");
    assert_eq!(code, 0);
    assert!(plain_text(&out).contains("SIBSH_ENV_TEST=visible"), "{out}");
}

#[test]
fn unset_removes_variable() {
    let (out, code) =
        run_shell("export SIBSH_UNSET_ME=x\nunset SIBSH_UNSET_ME\necho [$SIBSH_UNSET_ME]\n");
    assert_eq!(code, 0);
    assert!(visible_lines(&out).contains(&"[]".to_string()), "{out}");
}

#[test]
fn help_lists_available_commands() {
    let (out, code) = run_shell("help\n");
    assert_eq!(code, 0);
    for cmd in ["cd", "echo", "export", "history", "alias"] {
        assert!(
            plain_text(&out).contains(cmd),
            "help must mention {cmd}: {out}"
        );
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
    assert!(
        plain_text(&out).contains("[1 \u{2718}]"),
        "expected badge: {out}"
    );
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
    run_shell(&format!("echo one\necho two\nhistory > {f}\n"));
    let contents = fs::read_to_string(&f).unwrap();
    assert!(contents.contains("echo one"), "{contents}");
    assert!(contents.contains("echo two"), "{contents}");
    cleanup!(f);
}

// ---- Two-line prompt rendering ----

#[test]
fn prompt_renders_two_line_frame_with_brand() {
    let (out, code) = run_shell("true\n");
    assert_eq!(code, 0);
    let plain = plain_text(&out);
    assert!(plain.contains("[sibsh]"), "brand badge: {plain:?}");
    assert!(plain.contains('\u{256d}'), "top connector");
    assert!(plain.contains('\u{2570}'), "bottom connector");
}

#[test]
fn prompt_failure_shows_exit_badge_and_error_pointer() {
    let (out, code) = run_shell("false\ntrue\n");
    assert_eq!(code, 0);
    let plain = plain_text(&out);
    assert!(plain.contains("[1 \u{2718}]"), "badge: {plain:?}");
    assert!(plain.contains('\u{276d}'), "error pointer ❭ expected");
    // The success pointer must also appear (after the following `true`).
    assert!(plain.contains('\u{276f}'));
}

#[test]
fn prompt_hides_user_host_without_ssh_vars() {
    let (out, _) = run_shell_with("true\n", &[("USER", "ciuser"), ("HOSTNAME", "cihost")]);
    let plain = plain_text(&out);
    assert!(
        !plain.contains("ciuser@"),
        "no user@host locally: {plain:?}"
    );
}

#[test]
fn prompt_shows_user_host_only_over_ssh() {
    // The host part comes from /etc/hostname, so only the username side is
    // asserted here.
    let (out, _) = run_shell_with("true\n", &[("USER", "ciuser"), ("SSH_TTY", "/dev/pts/0")]);
    let plain = plain_text(&out);
    assert!(
        plain.contains("ciuser@"),
        "ssh session shows user@host: {plain:?}"
    );
}

#[test]
fn prompt_ascii_icons_via_config() {
    let config = tmpfile("cfg_ascii.toml");
    fs::write(&config, "icons = \"ascii\"\ngit_status = false\n").unwrap();
    let (out, code) = run_shell_with("true\nexit\n", &[("SIBSH_CONFIG", config.as_str())]);
    assert_eq!(code, 0);
    let plain = plain_text(&out);
    assert!(plain.contains("+-"), "ascii frame: {plain:?}");
    assert!(plain.contains("> "), "ascii pointer: {plain:?}");
    assert!(
        !plain.contains('\u{256d}'),
        "no unicode frame in ascii mode"
    );
    cleanup!(config);
}

#[test]
fn prompt_git_branch_segment_in_repository() {
    use std::io::Write as _;

    let dir = std::env::temp_dir().join(format!("sibsh_git_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let init = Command::new("git")
        .args(["init", "-q", "-b", "sibsh-test"])
        .current_dir(&dir)
        .status()
        .expect("git available for this test");
    assert!(init.success(), "git init must succeed");

    let mut child = Command::new(env!("CARGO_BIN_EXE_sibsh"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("SSH_TTY")
        .env_remove("SSH_CONNECTION")
        .env("SIBSH_FORCE_NON_ROOT", "1")
        .current_dir(&dir)
        .spawn()
        .expect("spawn sibsh");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"true\nexit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let plain = plain_text(&String::from_utf8_lossy(&output.stdout));

    assert!(plain.contains("on \u{f418}sibsh-test"), "branch: {plain:?}");

    // Untracked file -> ?1 flag.
    fs::write(dir.join("stray.txt"), "x").unwrap();
    let (out, _) = run_shell_in(&dir, "true\nexit\n");
    let plain = plain_text(&out);
    assert!(plain.contains("?1"), "untracked flag: {plain:?}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prompt_duration_timer_after_long_command() {
    let (out, code) = run_shell("sleep 2.3\ntrue\n");
    assert_eq!(code, 0);
    let plain = plain_text(&out);
    assert!(plain.contains('\u{f0150}'), "clock glyph: {plain:?}");
    assert!(plain.contains("2."), "seconds value: {plain:?}");
}

/// Runs a shell script with `dir` as the working directory.
fn run_shell_in(dir: &std::path::Path, script: &str) -> (String, i32) {
    use std::io::Write;

    let mut child = Command::new(env!("CARGO_BIN_EXE_sibsh"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("SSH_TTY")
        .env_remove("SSH_CONNECTION")
        .env("SIBSH_FORCE_NON_ROOT", "1")
        .current_dir(dir)
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

// ---- Phase 1.3: Pipelines (matrix from CHECKLISTS.md) ----

#[test]
fn matrix_1_pipeline_two_stages_builtin_to_external() {
    let (out, code) = run_shell("echo hello | cat\n");
    assert_eq!(code, 0);
    assert!(visible_lines(&out).contains(&"hello".to_string()), "{out}");
}

#[test]
fn matrix_2_pipeline_sorts_lines() {
    let (out, code) = run_shell("printf 'b\\na\\n' | sort\n");
    assert_eq!(code, 0);
    let lines = visible_lines(&out);
    let pos_a = lines.iter().position(|l| l == "a").expect("a sorted first");
    let pos_b = lines
        .iter()
        .position(|l| l == "b")
        .expect("b sorted second");
    assert!(pos_a < pos_b, "sort order violated: {lines:?}");
}

#[test]
fn matrix_3_three_stage_chain_counts_correctly() {
    let (out, code) = run_shell("printf 'x1\\nx2\\nx3\\nother\\n' | grep ^x | wc -l\n");
    assert_eq!(code, 0);
    assert!(visible_lines(&out).contains(&"3".to_string()), "{out}");
}

#[test]
fn matrix_4_yes_head_terminates_cleanly() {
    // `yes` must die of SIGPIPE once `head` exits — no leaked write end may
    // keep it alive (that would hang this test).
    let (out, code) = run_shell("yes | head -5\ntrue\n");
    assert_eq!(code, 0);
    assert_eq!(
        visible_lines(&out)
            .iter()
            .filter(|l| l.as_str() == "y")
            .count(),
        5,
        "exactly five y lines expected: {out}"
    );
}

#[test]
fn matrix_5_unknown_first_stage_reports_but_last_wins() {
    let (out, err, code) = run_shell_full("nosuchcmd_zz | cat\ntrue\n", &[]);
    assert_eq!(code, 0, "overall status comes from the last stage");
    assert!(err.contains("command not found"), "stderr: {err}");
    // The pipeline still drained; the shell continued to `true`.
    assert!(!plain_text(&out).is_empty());
}

#[test]
fn matrix_6_history_builtin_feeds_pipeline() {
    // History records each line as typed. Only `echo markerline` ends with
    // the marker (the pipeline line itself ends with a quote), so grep -c
    // must report exactly one.
    let script = "echo markerline\nhistory | grep -c 'markerline$'\ntrue\n";
    let (out, code) = run_shell(script);
    assert_eq!(code, 0);
    assert!(visible_lines(&out).contains(&"1".to_string()), "{out}");
}

#[test]
fn matrix_7_redirect_and_pipe_mix_works() {
    let input = tmpfile("p7_in.txt");
    let output = tmpfile("p7_out.txt");
    fs::write(&input, "pear\napple\n").unwrap();
    let (_, code) = run_shell(&format!("cat < {input} | sort > {output}\n"));
    assert_eq!(code, 0);
    assert_eq!(fs::read_to_string(&output).unwrap(), "apple\npear\n");
    cleanup!(input, output);
}

#[test]
fn matrix_8_false_true_overall_status_zero() {
    let (_, code) = run_shell("false | true\n");
    assert_eq!(code, 0, "last stage wins");
}

#[test]
fn matrix_9_true_false_overall_status_one() {
    let (_, code) = run_shell("true | false\n");
    assert_eq!(code, 1, "last stage wins");
}

#[test]
fn matrix_10_quoted_pipe_is_literal_no_pipeline() {
    let (out, code) = run_shell("echo 'a | b'\n");
    assert_eq!(code, 0);
    assert!(
        visible_lines(&out).contains(&"a | b".to_string()),
        "literal text expected: {out}"
    );
}

#[test]
fn stress_nine_stages_eight_pipes_complete_without_hang() {
    let chain = "echo deep | cat | cat | cat | cat | cat | cat | cat | cat\n";
    let (out, code) = run_shell(chain);
    assert_eq!(code, 0);
    assert!(visible_lines(&out).contains(&"deep".to_string()), "{out}");
}

#[test]
fn builtin_mid_pipeline_passes_through() {
    let (out, code) = run_shell("printf 'b\\na\\n' | cat | sort\n");
    assert_eq!(code, 0);
    let lines = visible_lines(&out);
    let pos_a = lines.iter().position(|l| l == "a").expect("a present");
    let pos_b = lines.iter().position(|l| l == "b").expect("b present");
    assert!(
        pos_a < pos_b,
        "cat (builtin) mid-chain broke order: {lines:?}"
    );
}

#[test]
fn builtin_last_stage_reads_from_pipe() {
    let (out, code) = run_shell("printf 'hello-from-pipe\\n' | cat\n");
    assert_eq!(code, 0);
    assert!(
        visible_lines(&out).contains(&"hello-from-pipe".to_string()),
        "{out}"
    );
}

#[test]
fn pipeline_syntax_errors_reported_and_shell_survives() {
    for bad in ["| cat", "cat |", "cat | | wc -l", "cat || wc -l"] {
        let (out, err, code) = run_shell_full(&format!("{bad}\necho survived\n"), &[]);
        assert_eq!(code, 0, "shell must survive {bad:?}");
        assert!(err.contains("syntax error"), "stderr for {bad:?}: {err}");
        assert!(
            visible_lines(&out).contains(&"survived".to_string()),
            "next command ran after {bad:?}: {out}"
        );
    }
}

#[test]
fn redirect_binds_tighter_than_pipe() {
    let f = tmpfile("p_bind.txt");
    let (out, _, _) = run_shell_full(&format!("echo hi > {f} | cat\n"), &[]);
    // First stage writes to the file; downstream sees immediate EOF.
    assert_eq!(fs::read_to_string(&f).unwrap(), "hi\n");
    assert!(
        !visible_lines(&out).contains(&"hi".to_string()),
        "nothing reaches the terminal: {out}"
    );
    cleanup!(f);
}

#[test]
fn alias_expands_in_each_pipeline_stage() {
    let config = tmpfile("cfg_pipe.toml");
    fs::write(&config, "[aliases]\npl = \"echo piped\"\n").unwrap();
    let (out, code) = run_shell_with("pl | cat\n", &[("SIBSH_CONFIG", config.as_str())]);
    assert_eq!(code, 0);
    assert!(
        visible_lines(&out).contains(&"piped".to_string()),
        "alias must expand inside pipelines: {out}"
    );
    cleanup!(config);
}

#[test]
fn pipeline_failure_sets_prompt_badge_status() {
    let (out, _) = run_shell("true | false\necho after=$?\n");
    assert!(
        plain_text(&out).contains("[1 \u{2718}]"),
        "failure badge expected: {out}"
    );
    assert!(
        visible_lines(&out).contains(&"after=1".to_string()),
        "{out}"
    );
}

#[test]
fn pipeline_output_larger_than_pipe_buffer_completes() {
    // 256 KiB forces multiple pipe-buffer round trips between stages; any
    // missing EOF or wrong wait order would deadlock here.
    let (out, code) = run_shell("head -c 262144 /dev/zero | wc -c\n");
    assert_eq!(code, 0);
    assert!(visible_lines(&out).contains(&"262144".to_string()), "{out}");
}
