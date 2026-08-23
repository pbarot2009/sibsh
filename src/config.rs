//! Runtime configuration loaded from `~/.sibsh/sibsh.toml`.
//!
//! Uses a small TOML-subset parser (std only, no external crates). Supported
//! syntax: `[section]` headers, `key = "string"`, `key = 123`, `key = true`,
//! and `key = ["a", "b"]` arrays of strings. Comments start with `#`.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

/// Parsed shell configuration.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Prompt template. Placeholders: `{user}`, `{host}`, `{cwd}`, `{status}`.
    pub prompt: Option<String>,
    /// Maximum number of history entries kept in memory.
    pub history_limit: Option<usize>,
    /// Command aliases from the `aliases` table.
    pub aliases: Vec<(String, String)>,
    /// Shell files (bashrc/zshrc style) executed at startup.
    pub imports: Vec<String>,
}

/// Flat key/value map with section prefixes, e.g. `aliases.ll` -> `"ls -la"`.
type Table = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    Array(Vec<String>),
}

impl Config {
    /// Commented template written to `$HOME/.sibsh/sibsh.toml` on first run
    /// so users can see every supported option and how to use it.
    pub const TEMPLATE: &str = r#"# sibsh configuration file
# Location: ~/.sibsh/sibsh.toml (override with the SIBSH_CONFIG env var)
# Syntax: TOML subset - strings "like this", integers, booleans,
# arrays of strings ["a", "b"], [sections], and # comments.
#
# Every key below is optional. Delete what you do not need;
# a missing or invalid file never stops the shell from starting.

# ------------------------------------------------------------------
# prompt: custom prompt text. Available placeholders:
#   {user}   your username          {host}  machine hostname
#   {cwd}    current directory      {status} last exit code (0 on success)
# Default (when this line is removed):
#   "{user}@{host}:{cwd} ❯ " with colors and a red [status] on failure
# ------------------------------------------------------------------
#prompt = "{user}@{host}:{cwd} ❯ "

# ------------------------------------------------------------------
# history_limit: maximum number of commands kept in memory this session.
# Oldest entries are dropped first. Remove the line for unlimited.
# ------------------------------------------------------------------
history_limit = 1000

# ------------------------------------------------------------------
# imports: shell files executed once at startup, before the first prompt,
# bashrc/zshrc style. Each line of each file runs through sibsh:
#
#   WORKS:  export KEY=VALUE      (set environment variables)
#           alias name='value'    (define aliases)
#           plain commands        (echo hello, cd /tmp, ...)
#           # comments            (and blank lines are skipped)
#
#   SKIPPED SILENTLY: bash-only syntax sibsh cannot parse, such as
#           if/then/fi, case/esac, for loops, functions, [[ ]] tests,
#           command substitution $(...), source, ulimit, stty ...
#
# This means you can point imports at your real ~/.bashrc or ~/.zshrc
# and sibsh picks up the exports and aliases it understands.
# Paths support a leading ~/ for your home directory.
# ------------------------------------------------------------------
#imports = ["~/.bashrc", "~/.zshrc"]

# ------------------------------------------------------------------
# aliases: startup shortcuts. Same effect as typing
#   alias ll='ls -la'
# at the prompt. Runtime `alias`/`unalias` work too but are not saved
# back to this file. The first word of a command expands once.
# ------------------------------------------------------------------
[aliases]
ll = "ls -la"
la = "ls -a"
gs = "git status"
"#;

    /// Config file path: `$SIBSH_CONFIG` if set, else `$HOME/.sibsh/sibsh.toml`.
    pub fn path() -> Option<PathBuf> {
        if let Ok(path) = env::var("SIBSH_CONFIG") {
            return Some(PathBuf::from(path));
        }
        env::var("HOME").ok().map(|home| {
            PathBuf::from(home).join(".sibsh").join("sibsh.toml")
        })
    }

    /// Writes the commented template when the default config file does not
    /// exist yet, so a fresh install has a self-documenting starting point.
    /// Only touches the default `$HOME/.sibsh/sibsh.toml` location; an
    /// explicit `$SIBSH_CONFIG` override is never created or modified.
    pub fn ensure_default() {
        if env::var_os("SIBSH_CONFIG").is_some() {
            return;
        }
        let Some(home) = env::var_os("HOME") else {
            return;
        };
        let dir = PathBuf::from(&home).join(".sibsh");
        let path = dir.join("sibsh.toml");
        if path.exists() {
            return;
        }
        if fs::create_dir_all(&dir).is_ok() {
            let _ = fs::write(path, Self::TEMPLATE);
        }
    }

    /// Loads the config file. A missing or unreadable file yields the default
    /// config (the shell must always start).
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match fs::read_to_string(&path) {
            Ok(text) => {
                let config = Self::from_str(&text);
                if let Err(msg) = &config {
                    eprintln!("sibsh: config {}: {msg}", path.display());
                }
                config.unwrap_or_default()
            }
            // A missing file is normal; other read errors are worth reporting.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                eprintln!("sibsh: config {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// Parses config text. Returns an error message on invalid syntax.
    pub fn from_str(text: &str) -> Result<Self, String> {
        let table = parse_table(text)?;
        let mut config = Self::default();

        if let Some(Value::Str(s)) = table.get("prompt") {
            config.prompt = Some(s.clone());
        }
        if let Some(Value::Int(n)) = table.get("history_limit") {
            config.history_limit = Some(usize::try_from(*n).unwrap_or(0));
        }
        if let Some(Value::Array(items)) = table.get("imports") {
            config.imports.clone_from(items);
        }
        for (key, value) in &table {
            let Some(name) = key.strip_prefix("aliases.") else {
                continue;
            };
            if let Value::Str(s) = value {
                config.aliases.push((name.to_string(), s.clone()));
            }
        }

        Ok(config)
    }
}

/// Expands a leading `~` to `$HOME` (`~`, `~/x`). Other input is returned as-is.
pub fn expand_tilde(path: &str) -> String {
    let Some(rest) = path.strip_prefix('~') else {
        return path.to_string();
    };
    let home = env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return path.to_string();
    }
    match rest {
        "" => home,
        p if p.starts_with('/') => format!("{home}{p}"),
        _ => path.to_string(), // `~user` forms are not supported yet
    }
}

/// Parses the TOML subset into a flat table.
fn parse_table(text: &str) -> Result<Table, String> {
    let mut table = Table::new();
    let mut section = String::new();

    for (idx, raw_line) in text.lines().enumerate() {
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        let lineno = idx + 1;

        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let name = name.trim();
            if name.is_empty() {
                return Err(format!("line {lineno}: empty section name"));
            }
            section = format!("{name}.");
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {lineno}: expected `key = value`, got `{line}`"));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("line {lineno}: empty key"));
        }
        let value = parse_value(value.trim())
            .map_err(|msg| format!("line {lineno}: {msg}"))?;
        table.insert(format!("{section}{key}"), value);
    }

    Ok(table)
}

/// Removes a trailing `#` comment that is not inside a quoted string.
fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let chars = line.char_indices();
    for (idx, ch) in chars {
        match ch {
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..idx],
            _ => {}
        }
    }
    line
}

fn parse_value(text: &str) -> Result<Value, String> {
    if text.is_empty() {
        return Err("missing value".to_string());
    }
    if let Some(rest) = text.strip_prefix('"') {
        let Some((content, rest)) = rest.split_once('"') else {
            return Err("unterminated string".to_string());
        };
        if !rest.trim().is_empty() {
            return Err("unexpected text after string".to_string());
        }
        return Ok(Value::Str(content.to_string()));
    }
    if text == "true" {
        return Ok(Value::Bool(true));
    }
    if text == "false" {
        return Ok(Value::Bool(false));
    }
    if let Ok(n) = text.parse::<i64>() {
        return Ok(Value::Int(n));
    }
    if let Some(rest) = text.strip_prefix('[') {
        let Some(items_text) = rest.strip_suffix(']') else {
            return Err("unterminated array".to_string());
        };
        let mut items = Vec::new();
        for item in items_text.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            match parse_value(item)? {
                Value::Str(s) => items.push(s),
                _ => return Err("array items must be strings".to_string()),
            }
        }
        return Ok(Value::Array(items));
    }
    Err(format!("unsupported value `{text}` (use string, number, bool, or string array)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_supported_types() {
        let config = Config::from_str(
            "prompt = \"[{user}@{host} {cwd}] \"\n\
             history_limit = 500\n\
             imports = [\"~/.bashrc\", \"~/.zshrc\"]\n",
        )
        .expect("valid config");
        assert_eq!(config.prompt.as_deref(), Some("[{user}@{host} {cwd}] "));
        assert_eq!(config.history_limit, Some(500));
        assert_eq!(config.imports, vec!["~/.bashrc", "~/.zshrc"]);
    }

    #[test]
    fn parses_aliases_table() {
        let config = Config::from_str("[aliases]\nll = \"ls -la\"\ngs = \"git status\"\n")
            .expect("valid config");
        // Keys are stored in a sorted map, so aliases come back sorted by name.
        let mut actual = config.aliases.clone();
        actual.sort();
        assert_eq!(
            actual,
            vec![
                ("gs".to_string(), "git status".to_string()),
                ("ll".to_string(), "ls -la".to_string()),
            ]
        );
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        let config = Config::from_str(
            "# full-line comment\n\nprompt = \"x\" # trailing comment\n# more\n",
        )
        .expect("valid config");
        assert_eq!(config.prompt.as_deref(), Some("x"));
    }

    #[test]
    fn hash_inside_string_kept() {
        let config = Config::from_str("prompt = \"a # b\"\n").expect("valid config");
        assert_eq!(config.prompt.as_deref(), Some("a # b"));
    }

    #[test]
    fn invalid_syntax_reports_line() {
        let err = Config::from_str("prompt =\n").unwrap_err();
        assert!(err.contains("line 1"), "got: {err}");

        let err = Config::from_str("noequals\n").unwrap_err();
        assert!(err.contains("line 1"), "got: {err}");

        let err = Config::from_str("prompt = \"unterminated\n").unwrap_err();
        assert!(err.contains("line 1"), "got: {err}");
    }

    #[test]
    fn empty_text_gives_default() {
        let config = Config::from_str("").expect("valid config");
        assert!(config.prompt.is_none());
        assert!(config.aliases.is_empty());
        assert!(config.imports.is_empty());
    }

    #[test]
    fn expand_tilde_forms() {
        // SAFETY: single-threaded test manipulating its own env var.
        unsafe {
            env::set_var("HOME", "/home/tester");
        }
        assert_eq!(expand_tilde("~"), "/home/tester");
        assert_eq!(expand_tilde("~/docs"), "/home/tester/docs");
        assert_eq!(expand_tilde("~other"), "~other");
        assert_eq!(expand_tilde("/abs/path"), "/abs/path");
    }

    #[test]
    fn expand_tilde_without_home_stays_literal() {
        // SAFETY: single-threaded test manipulating its own env var.
        unsafe {
            env::remove_var("HOME");
        }
        assert_eq!(expand_tilde("~"), "~");
        assert_eq!(expand_tilde("~/x"), "~/x");
        // SAFETY: restoring state for other tests.
        unsafe {
            env::set_var("HOME", "/home/tester");
        }
    }

    #[test]
    fn unknown_keys_and_sections_are_ignored() {
        let config = Config::from_str(
            "unknown_key = \"x\"\nnested = true\n[some_table]\nfoo = 1\n",
        )
        .expect("valid config");
        assert!(config.prompt.is_none());
        assert!(config.aliases.is_empty());
        assert!(config.imports.is_empty());
    }

    #[test]
    fn non_string_values_in_known_keys_are_ignored() {
        // A wrong type must not crash or misparse; the key falls back to default.
        let config = Config::from_str(
            "prompt = 42\nhistory_limit = \"many\"\nimports = \"not-an-array\"\n",
        )
        .expect("valid config");
        assert!(config.prompt.is_none());
        assert_eq!(config.history_limit, None);
        assert!(config.imports.is_empty());
    }

    #[test]
    fn negative_and_zero_history_limit_parse() {
        let config = Config::from_str("history_limit = -5\n").expect("valid");
        assert_eq!(config.history_limit, Some(0)); // clamped by usize conversion
        let config = Config::from_str("history_limit = 0\n").expect("valid");
        assert_eq!(config.history_limit, Some(0));
    }

    #[test]
    fn duplicate_key_last_wins() {
        let config = Config::from_str("history_limit = 10\nhistory_limit = 20\n").expect("valid");
        assert_eq!(config.history_limit, Some(20));
    }

    #[test]
    fn inline_comment_after_number_and_bool() {
        let config =
            Config::from_str("history_limit = 250 # capped\nflag = true # ignored key\n")
                .expect("valid");
        assert_eq!(config.history_limit, Some(250));
    }

    #[test]
    fn array_with_trailing_comma_and_spaces() {
        let config =
            Config::from_str("imports = [ \"~/a.sh\" , \"~/b.sh\" , ]\n").expect("valid");
        assert_eq!(config.imports, vec!["~/a.sh", "~/b.sh"]);
    }

    #[test]
    fn array_of_non_strings_is_error() {
        let err = Config::from_str("imports = [1, 2]\n").unwrap_err();
        assert!(err.contains("array items must be strings"), "got: {err}");
    }

    #[test]
    fn unterminated_section_header_is_value_error() {
        // `[aliases` without closing bracket parses as a `key = value` failure.
        let err = Config::from_str("[aliases\n").unwrap_err();
        assert!(err.contains("expected `key = value`"), "got: {err}");
    }

    #[test]
    fn empty_section_name_is_error() {
        let err = Config::from_str("[]\n").unwrap_err();
        assert!(err.contains("empty section name"), "got: {err}");
    }

    #[test]
    fn error_messages_carry_line_numbers() {
        let err = Config::from_str("# ok\n\nbad line here\n").unwrap_err();
        assert!(err.contains("line 3"), "got: {err}");
    }

    #[test]
    fn crlf_line_endings_accepted() {
        let config = Config::from_str("prompt = \"win\"\r\nhistory_limit = 3\r\n").expect("valid");
        assert_eq!(config.prompt.as_deref(), Some("win"));
        assert_eq!(config.history_limit, Some(3));
    }

    #[test]
    fn backslashes_inside_strings_are_literal() {
        // The subset parser does not process backslash escapes; content stays verbatim.
        let config = Config::from_str("prompt = \"a\\nb\"\n").expect("valid");
        assert_eq!(config.prompt.as_deref(), Some("a\\nb"));
    }

    #[test]
    fn escaped_quote_is_a_parse_error_documented_limitation() {
        // No escape processing means `\"` closes the string; the leftover text
        // makes the line invalid. This is a documented limitation of the subset.
        let err = Config::from_str("prompt = \"a\\\"b\"\n").unwrap_err();
        assert!(err.contains("line 1"), "got: {err}");
    }

    #[test]
    fn template_parses_as_valid_config() {
        // The shipped first-run template must always parse cleanly.
        let config =
            Config::from_str(Config::TEMPLATE).expect("template must be valid TOML-subset");
        assert_eq!(config.history_limit, Some(1000));
        assert!(config.prompt.is_none(), "template prompt stays commented out");
        assert!(config.imports.is_empty(), "template imports stay commented out");
        assert!(config.aliases.contains(&("ll".to_string(), "ls -la".to_string())));
        assert!(config.aliases.contains(&("gs".to_string(), "git status".to_string())));
    }
}
