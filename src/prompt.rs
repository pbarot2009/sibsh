//! Two-line, segment-based prompt in the style of starship and oh-my-posh:
//!
//! ```text
//! ╭─ [sibsh] user@host ~/code/project on  main [⇡2 ⇣1 !3 ?1] via  rs
//! ╰─❯
//! ```
//!
//! Each segment renders only when its data is available. All colors come
//! from the fixed ANSI-256 palette below; glyphs have an ASCII fallback
//! selected with `icons = "ascii"` in the config file.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// ANSI-256 color palette for the prompt.
mod clr {
    pub const FRAME: &str = "\x1b[38;5;242m"; // structural connectors
    pub const BRAND: &str = "\x1b[1;38;5;141m"; // [sibsh] badge
    pub const DIR: &str = "\x1b[1;38;5;75m"; // working directory
    pub const GIT: &str = "\x1b[38;5;211m"; // branch name
    pub const GIT_FLAG: &str = "\x1b[1;38;5;214m"; // dirty-state indicators
    pub const LANG: &str = "\x1b[38;5;114m"; // language tag
    pub const TIME: &str = "\x1b[38;5;222m"; // execution timer
    pub const ERR: &str = "\x1b[1;38;5;203m"; // failure code
    pub const OK_PTR: &str = "\x1b[1;38;5;120m"; // ready pointer
    pub const ROOT_PTR: &str = "\x1b[1;38;5;214m"; // root pointer
    pub const RESET: &str = "\x1b[0m";
}

/// Which glyph set the prompt draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconMode {
    /// Nerd Font / Unicode symbols (default).
    Nerd,
    /// Plain ASCII fallbacks for terminals without a patched font.
    Ascii,
}

impl IconMode {
    pub fn from_config(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some(v) if v.eq_ignore_ascii_case("ascii") => Self::Ascii,
            _ => Self::Nerd,
        }
    }
}

struct Glyphs {
    top: &'static str,
    bottom: &'static str,
    ok_ptr: &'static str,
    err_ptr: &'static str,
    root_ptr: &'static str,
    branch: &'static str,
    untracked: &'static str,
    modified: &'static str,
    ahead: &'static str,
    behind: &'static str,
    clock: &'static str,
    fail: &'static str,
    rust: &'static str,
    go: &'static str,
    c_cpp: &'static str,
}

impl Glyphs {
    fn new(mode: IconMode) -> Self {
        match mode {
            IconMode::Nerd => Self {
                top: "\u{256d}\u{2500}",    // ╭─
                bottom: "\u{2570}\u{2500}", // ╰─
                ok_ptr: "\u{276f}",         // ❯
                err_ptr: "\u{276d}",        // ❭
                root_ptr: "#",
                branch: "\u{f418}",
                untracked: "?",
                modified: "!",
                ahead: "\u{21e1}",  // ⇡
                behind: "\u{21e3}", // ⇣
                clock: "\u{f0150}", // 󰅐
                fail: "\u{2718}",   // ✘
                rust: "\u{f1617}",  // 󱘗
                go: "\u{f07d3}",    // 󰟓
                c_cpp: "\u{f0672}", // 󰙲
            },
            IconMode::Ascii => Self {
                top: "+-",
                bottom: "+-",
                ok_ptr: ">",
                err_ptr: ">",
                root_ptr: "#",
                branch: "git:",
                untracked: "?",
                modified: "!",
                ahead: "^",
                behind: "v",
                clock: "s",
                fail: "x",
                rust: "rs",
                go: "go",
                c_cpp: "c",
            },
        }
    }
}

pub struct PromptCtx {
    pub last_status: i32,
    pub last_duration: Option<Duration>,
    pub icons: IconMode,
    pub git_enabled: bool,
    pub root: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GitInfo {
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    pub modified: usize,
    pub untracked: usize,
}

const DURATION_MIN: Duration = Duration::from_millis(2000);
const PATH_MAX_COMPONENTS: usize = 3;

pub struct Prompt;

impl Prompt {
    pub fn render(ctx: &PromptCtx) -> String {
        let g = Glyphs::new(ctx.icons);
        let root = ctx.root;
        let failed = ctx.last_status != 0;
        let (_, _, dir_display) = identity();

        let mut segs: Vec<String> = Vec::new();

        segs.push(format!(
            "{brand}[sibsh]{reset}",
            brand = clr::BRAND,
            reset = clr::RESET,
        ));

        if root {
            segs.push(format!(
                "{ptr}#{reset}",
                ptr = clr::ROOT_PTR,
                reset = clr::RESET,
            ));
        } else if is_ssh_session() {
            let (user, host, _) = identity();
            segs.push(format!(
                "{clr}{user}@{host}{reset}",
                clr = clr::DIR,
                reset = clr::RESET,
            ));
        }

        segs.push(format!(
            "{clr}{dir_display}{reset}",
            clr = clr::DIR,
            reset = clr::RESET,
        ));

        if !root
            && ctx.git_enabled
            && let Some(info) = git_info()
        {
            segs.push(git_segment(&info, &g));
        }
        if !root {
            if let Some(runtime) = detect_runtime() {
                segs.push(format!(
                    "{clr}via {icon} {label}{reset}",
                    clr = clr::LANG,
                    icon = runtime.glyph(&g),
                    label = runtime.label(),
                    reset = clr::RESET,
                ));
            }
            if let Some(text) = duration_text(ctx.last_duration) {
                segs.push(format!(
                    "{clr}[{text} {clock}]{reset}",
                    clr = clr::TIME,
                    clock = g.clock,
                    reset = clr::RESET,
                ));
            }
            if failed {
                segs.push(format!(
                    "{clr}[{code} {fail}]{reset}",
                    clr = clr::ERR,
                    code = ctx.last_status,
                    fail = g.fail,
                    reset = clr::RESET,
                ));
            }
        }

        let top = format!(
            "{frame}{top_glyph}{reset} {segs}",
            frame = clr::FRAME,
            top_glyph = g.top,
            reset = clr::RESET,
            segs = segs.join(" "),
        );

        let (pointer_color, pointer) = match (root, failed) {
            (true, _) => (clr::ROOT_PTR, g.root_ptr),
            (_, true) => (clr::ERR, g.err_ptr),
            (_, false) => (clr::OK_PTR, g.ok_ptr),
        };
        format!(
            "{top}\n{frame}{bottom}{reset} {ptr_color}{pointer}{reset} ",
            frame = clr::FRAME,
            bottom = g.bottom,
            reset = clr::RESET,
            ptr_color = pointer_color,
        )
    }

    pub fn render_with(template: &str, ctx: &PromptCtx) -> String {
        let (_, _, display_path) = identity();
        let branch = git_info().map_or_else(String::new, |info| info.branch);
        template
            .replace("{user}", &identity().0)
            .replace("{host}", &identity().1)
            .replace("{cwd}", &display_path)
            .replace("{status}", &ctx.last_status.to_string())
            .replace("{branch}", &branch)
            + " "
    }

    pub fn render_auto(template: Option<&str>, ctx: &PromptCtx) -> String {
        match template {
            Some(t) if !t.trim().is_empty() => Self::render_with(t, ctx),
            _ => Self::render(ctx),
        }
    }
}

fn identity() -> (String, String, String) {
    let user = env::var("USER")
        .or_else(|_| env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_string());

    let hostname = fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .or_else(|_| env::var("HOSTNAME"))
        .unwrap_or_else(|_| "localhost".to_string());

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let display_path = display_path(&cwd);

    (user, hostname, display_path)
}

fn display_path(cwd: &Path) -> String {
    let home = env::var("HOME").unwrap_or_default();
    let raw = cwd.to_string_lossy();
    let shortened = if !home.is_empty() && raw.starts_with(home.as_str()) {
        raw.replacen(home.as_str(), "~", 1)
    } else {
        raw.into_owned()
    };
    truncate_path(&shortened, PATH_MAX_COMPONENTS)
}

fn truncate_path(path: &str, max_components: usize) -> String {
    let mut comps: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    if comps.len() <= max_components {
        return path.to_string();
    }
    let dropped = comps.split_off(comps.len() - max_components);
    let kept: Vec<String> = dropped.iter().map(|c| (*c).to_string()).collect();
    format!("…/{}", kept.join("/"))
}

fn duration_text(duration: Option<Duration>) -> Option<String> {
    let d = duration?;
    if d <= DURATION_MIN {
        return None;
    }
    let whole = d.as_secs();
    Some(if whole < 60 {
        format!("{:.2}s", d.as_secs_f64())
    } else {
        format!("{}m {}s", whole / 60, whole % 60)
    })
}

fn git_flags(info: &GitInfo, g: &Glyphs) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if info.ahead > 0 {
        parts.push(format!("{}{}", g.ahead, info.ahead));
    }
    if info.behind > 0 {
        parts.push(format!("{}{}", g.behind, info.behind));
    }
    if info.modified > 0 {
        parts.push(format!("{}{}", g.modified, info.modified));
    }
    if info.untracked > 0 {
        parts.push(format!("{}{}", g.untracked, info.untracked));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn git_segment(info: &GitInfo, g: &Glyphs) -> String {
    let flags_part = git_flags(info, g).map_or_else(String::new, |flags| {
        format!(
            " {flag}[{flags}]{reset}",
            flag = clr::GIT_FLAG,
            reset = clr::RESET,
        )
    });
    format!(
        "{clr}on {git}{branch}{name}{flags_part}{reset}",
        clr = clr::FRAME,
        git = clr::GIT,
        branch = g.branch,
        name = info.branch,
        reset = clr::RESET,
    )
}

fn is_ssh_session() -> bool {
    ["SSH_TTY", "SSH_CONNECTION"]
        .iter()
        .any(|k| env::var_os(k).is_some_and(|v| !v.is_empty()))
}

pub fn is_root() -> bool {
    if env::var_os("SIBSH_FORCE_NON_ROOT").is_some() {
        return false;
    }
    if let Ok(status) = fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("Uid:")
                && rest.split_whitespace().next() == Some("0")
            {
                return true;
            }
        }
    }
    env::var("USER").as_deref() == Ok("root") || env::var("HOME").as_deref() == Ok("/root")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Runtime {
    Rust,
    Go,
    CCpp,
}

impl Runtime {
    fn glyph(self, g: &Glyphs) -> &'static str {
        match self {
            Runtime::Rust => g.rust,
            Runtime::Go => g.go,
            Runtime::CCpp => g.c_cpp,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Runtime::Rust => "rs",
            Runtime::Go => "go",
            Runtime::CCpp => "c",
        }
    }
}

fn detect_runtime() -> Option<Runtime> {
    let cwd = env::current_dir().ok()?;
    if cwd.join("Cargo.toml").exists() {
        return Some(Runtime::Rust);
    }
    if cwd.join("go.mod").exists() {
        return Some(Runtime::Go);
    }
    if cwd.join("CMakeLists.txt").exists() || has_extension(&cwd, &["c", "h", "cc", "cpp", "hpp"]) {
        return Some(Runtime::CCpp);
    }
    if has_extension(&cwd, &["go"]) {
        return Some(Runtime::Go);
    }
    None
}

fn has_extension(dir: &Path, exts: &[&str]) -> bool {
    fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| exts.contains(&ext.to_string_lossy().as_ref()))
        })
    })
}

pub fn git_info() -> Option<GitInfo> {
    let cwd = env::current_dir().ok()?;
    let output = Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .arg("-b")
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_git_status(&text)
}

fn parse_git_status(text: &str) -> Option<GitInfo> {
    let mut info = GitInfo::default();
    let mut saw_branch_header = false;

    for line in text.lines() {
        if let Some(header) = line.strip_prefix("## ") {
            saw_branch_header = true;
            parse_branch_header(header, &mut info);
        } else if let Some((xy, _)) = line.split_once(' ') {
            if xy == "??" {
                info.untracked += 1;
            } else if xy != "!!" {
                info.modified += 1;
            }
        }
    }
    saw_branch_header.then_some(info)
}

fn parse_branch_header(header: &str, info: &mut GitInfo) {
    let (tracking_part, suffix) = match header.split_once(" [") {
        Some((left, right)) => (left, Some(right.trim_end_matches(']'))),
        None => (header, None),
    };

    let head_part = tracking_part.split("...").next().unwrap_or(tracking_part);
    let skip = usize::from(head_part.starts_with("No commits yet on ")) * 4;
    info.branch = head_part
        .split_whitespace()
        .nth(skip)
        .unwrap_or("HEAD")
        .to_string();

    if let Some(suffix) = suffix {
        for part in suffix.split(',') {
            let part = part.trim();
            if let Some(n) = part.strip_prefix("ahead ") {
                info.ahead = n.parse().unwrap_or(0);
            } else if let Some(n) = part.strip_prefix("behind ") {
                info.behind = n.parse().unwrap_or(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: &str) -> String {
        let mut out = String::new();
        let mut in_ansi = false;
        for ch in s.chars() {
            match ch {
                '\x1b' => in_ansi = true,
                'm' if in_ansi => in_ansi = false,
                _ if !in_ansi => out.push(ch),
                _ => {}
            }
        }
        out
    }

    fn ctx(status: i32) -> PromptCtx {
        PromptCtx {
            last_status: status,
            last_duration: None,
            icons: IconMode::Nerd,
            git_enabled: false,
            root: false,
        }
    }

    #[test]
    fn truncate_keeps_short_paths_exactly() {
        assert_eq!(truncate_path("~", 3), "~");
        assert_eq!(truncate_path("~/code/project", 3), "~/code/project");
        assert_eq!(truncate_path("/usr", 3), "/usr");
        assert_eq!(truncate_path("/usr/local/bin", 3), "/usr/local/bin");
    }

    #[test]
    fn truncate_contracts_deep_paths_to_three_segments() {
        assert_eq!(
            truncate_path("/home/u/code/compiler/parser/ast", 3),
            "…/compiler/parser/ast"
        );
        assert_eq!(truncate_path("~/a/b/c/d/e", 3), "…/c/d/e");
    }

    #[test]
    fn truncate_four_levels_drops_only_the_first() {
        assert_eq!(truncate_path("/one/two/three/four", 3), "…/two/three/four");
    }

    #[test]
    fn duration_hidden_under_two_seconds() {
        assert_eq!(duration_text(Some(Duration::from_millis(150))), None);
        assert_eq!(duration_text(Some(DURATION_MIN)), None);
        assert_eq!(duration_text(None), None);
    }

    #[test]
    fn duration_formats_seconds_and_minutes() {
        assert_eq!(
            duration_text(Some(Duration::from_millis(3420))),
            Some("3.42s".to_string())
        );
        assert_eq!(
            duration_text(Some(Duration::from_secs(125))),
            Some("2m 5s".to_string())
        );
    }

    #[test]
    fn git_flags_match_specification_order() {
        let g = Glyphs::new(IconMode::Nerd);
        let info = GitInfo {
            branch: "dev".into(),
            ahead: 2,
            behind: 1,
            modified: 3,
            untracked: 1,
        };
        assert_eq!(
            git_flags(&info, &g),
            Some("\u{21e1}2 \u{21e3}1 !3 ?1".into())
        );
    }

    #[test]
    fn git_flags_ascii_mode_uses_fallbacks() {
        let g = Glyphs::new(IconMode::Ascii);
        let info = GitInfo {
            branch: "dev".into(),
            ahead: 2,
            behind: 1,
            modified: 3,
            untracked: 1,
        };
        assert_eq!(git_flags(&info, &g), Some("^2 v1 !3 ?1".into()));
    }

    #[test]
    fn clean_synced_tree_has_no_flags() {
        let g = Glyphs::new(IconMode::Nerd);
        let info = GitInfo::default();
        assert_eq!(git_flags(&info, &g), None);
    }

    #[test]
    fn partial_flags_only_show_nonzero_counts() {
        let g = Glyphs::new(IconMode::Nerd);
        let info = GitInfo {
            branch: "main".into(),
            modified: 2,
            ..GitInfo::default()
        };
        assert_eq!(git_flags(&info, &g), Some("!2".into()));
    }

    #[test]
    fn git_status_parsing_full_example() {
        let text = "## dev...origin/dev [ahead 2, behind 1]\n\
                    M  src/a.rs\n\
                    ?? notes.txt\n\
                    ?? tmp/\n";
        let info = parse_git_status(text).expect("header present");
        assert_eq!(info.branch, "dev");
        assert_eq!(info.ahead, 2);
        assert_eq!(info.behind, 1);
        assert_eq!(info.modified, 1);
        assert_eq!(info.untracked, 2);
    }

    #[test]
    fn git_status_parsing_clean_tree() {
        let info = parse_git_status("## main...origin/main\n").expect("header");
        assert_eq!(info.branch, "main");
        assert_eq!(
            info,
            GitInfo {
                branch: "main".into(),
                ..Default::default()
            }
        );
    }

    #[test]
    fn git_status_parsing_no_upstream() {
        let info = parse_git_status("## feature-x\nM  f.txt\n").expect("header");
        assert_eq!(info.branch, "feature-x");
        assert_eq!(info.ahead, 0);
        assert_eq!(info.modified, 1);
    }

    #[test]
    fn git_status_parsing_fresh_repo() {
        let info = parse_git_status("## No commits yet on master\n?? x\n").expect("header");
        assert_eq!(info.branch, "master");
        assert_eq!(info.untracked, 1);
    }

    #[test]
    fn git_status_without_header_is_not_a_repo_summary() {
        assert_eq!(parse_git_status("M  file.txt\n"), None);
    }

    #[test]
    fn icon_mode_selection() {
        assert_eq!(IconMode::from_config(None), IconMode::Nerd);
        assert_eq!(IconMode::from_config(Some("nerd")), IconMode::Nerd);
        assert_eq!(IconMode::from_config(Some("ASCII")), IconMode::Ascii);
        assert_eq!(IconMode::from_config(Some("ascii")), IconMode::Ascii);
        assert_eq!(IconMode::from_config(Some("junk")), IconMode::Nerd);
    }

    #[test]
    fn render_contains_frame_brand_and_pointer() {
        let p = plain(&Prompt::render(&ctx(0)));
        assert!(p.contains("[sibsh]"));
        assert!(p.contains('\u{256d}'));
        assert!(p.contains('\u{2570}'));
        assert!(p.ends_with("\u{276f} "));
    }

    #[test]
    fn render_failure_shows_badge_and_error_pointer() {
        let p = plain(&Prompt::render(&ctx(127)));
        assert!(p.contains("[127 \u{2718}]"), "exit badge missing: {p:?}");
        assert!(p.ends_with("\u{276d} "));
    }

    #[test]
    fn render_root_mode_strips_segments_and_uses_hash_pointer() {
        let mut c = ctx(0);
        c.root = true;
        let p = plain(&Prompt::render(&c));
        assert!(p.ends_with("# "), "root pointer expected: {p:?}");
        assert!(!p.contains("via"), "root mode drops runtime noise: {p:?}");
    }

    #[test]
    fn render_ascii_mode_uses_fallback_glyphs() {
        let mut c = ctx(0);
        c.icons = IconMode::Ascii;
        let p = plain(&Prompt::render(&c));
        assert!(p.contains("+-"), "ascii connector missing: {p:?}");
        assert!(p.ends_with("> "));
    }

    #[test]
    fn render_timer_segment_appears_above_threshold() {
        let mut c = ctx(0);
        c.icons = IconMode::Ascii;
        c.last_duration = Some(Duration::from_millis(3420));
        let p = plain(&Prompt::render(&c));
        assert!(p.contains("[3.42s s]"), "timer segment missing: {p:?}");
    }

    #[test]
    fn custom_template_still_supported() {
        let c = ctx(3);
        let out = Prompt::render_auto(Some("sh:{cwd}:{status}"), &c);
        assert!(out.starts_with("sh:"), "custom template must win: {out:?}");
        assert!(out.contains(":3 "));
        assert!(Prompt::render_auto(Some("  "), &c).contains("[sibsh]"));
    }

    #[test]
    fn git_segment_renders_branch_and_flags() {
        let g = Glyphs::new(IconMode::Nerd);
        let info = GitInfo {
            branch: "dev".into(),
            ahead: 2,
            ..GitInfo::default()
        };
        let seg = plain(&git_segment(&info, &g));
        assert!(seg.contains("on \u{f418}dev"), "got: {seg:?}");
        assert!(seg.contains('\u{21e1}'));
    }
}
