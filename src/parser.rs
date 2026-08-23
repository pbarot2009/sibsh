use crate::error::{ShellError, ShellResult};
use std::env;

#[derive(Debug, PartialEq, Eq)]
enum ParseState {
    Normal,
    InSingleQuote,
    InDoubleQuote,
}

/// The direction and write behavior of an I/O redirection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectMode {
    /// `< file` — wire stdin to an existing file opened read-only.
    Input,
    /// `> file` — wire stdout to a file created or truncated.
    OutputTrunc,
    /// `>> file` — wire stdout to a file created or appended to.
    OutputAppend,
}

/// A single parsed redirection, e.g. `>> out.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirection {
    pub mode: RedirectMode,
    pub path: String,
}

/// The result of tokenizing one input line: the command words plus any
/// redirections, in the order they appeared. A pipeline line yields one
/// `ParsedCommand` per stage.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedCommand {
    pub args: Vec<String>,
    pub redirects: Vec<Redirection>,
}

impl ParsedCommand {
    /// True when the stage carries neither arguments nor redirections.
    pub fn is_empty(&self) -> bool {
        self.args.is_empty() && self.redirects.is_empty()
    }
}

pub struct Parser;

impl Parser {
    /// Tokenizes an input line into pipeline stages, expanding variables and
    /// stripping quotes. Operators (`>`, `>>`, `<`, `|`) are only recognized
    /// outside quotes, so `echo 'a > b'` stays a literal argument while
    /// `cat f | sort` splits into two stages. Each stage keeps its own
    /// redirections; a single-command line yields a one-element vector.
    pub fn parse(input: &str, last_status: i32) -> ShellResult<Vec<ParsedCommand>> {
        let mut stages: Vec<ParsedCommand> = vec![ParsedCommand::default()];
        let mut current_token = String::new();
        let mut state = ParseState::Normal;
        let mut pending_redirect: Option<RedirectMode> = None;
        let mut chars = input.chars().peekable();

        while let Some(ch) = chars.next() {
            match state {
                ParseState::Normal => match ch {
                    ' ' | '\t' | '\r' | '\n' => {
                        Self::flush_token(
                            stages.last_mut().expect("stage always present"),
                            &mut current_token,
                            &mut pending_redirect,
                        );
                    }
                    '\'' => {
                        state = ParseState::InSingleQuote;
                    }
                    '"' => {
                        state = ParseState::InDoubleQuote;
                    }
                    '\\' => {
                        if let Some(next_ch) = chars.next() {
                            current_token.push(next_ch);
                        }
                    }
                    '$' => {
                        let var_value = Self::extract_var(&mut chars, last_status);
                        current_token.push_str(&var_value);
                    }
                    '|' => {
                        Self::check_pipe(&mut chars, pending_redirect, &current_token)?;
                        // The operator terminates the current word and closes
                        // the stage (`echo hi|wc -c` works without spaces).
                        Self::flush_token(
                            stages.last_mut().expect("stage always present"),
                            &mut current_token,
                            &mut pending_redirect,
                        );
                        stages.push(ParsedCommand::default());
                    }
                    '<' => {
                        Self::start_redirect(
                            stages.last_mut().expect("stage always present"),
                            &mut current_token,
                            &mut pending_redirect,
                            RedirectMode::Input,
                        )?;
                    }
                    '>' => {
                        let mode = if chars.peek() == Some(&'>') {
                            chars.next();
                            RedirectMode::OutputAppend
                        } else {
                            RedirectMode::OutputTrunc
                        };
                        Self::start_redirect(
                            stages.last_mut().expect("stage always present"),
                            &mut current_token,
                            &mut pending_redirect,
                            mode,
                        )?;
                    }
                    _ => {
                        current_token.push(ch);
                    }
                },
                ParseState::InSingleQuote => match ch {
                    '\'' => {
                        state = ParseState::Normal;
                    }
                    _ => {
                        current_token.push(ch);
                    }
                },
                ParseState::InDoubleQuote => {
                    if Self::double_quote_step(ch, &mut chars, &mut current_token, last_status) {
                        state = ParseState::Normal;
                    }
                }
            }
        }

        if state != ParseState::Normal {
            return Err(ShellError::ParseError(
                "unclosed quote detected".to_string(),
            ));
        }

        // Commit a trailing word (redirection target or final argument) and
        // reject dangling operators.
        Self::finish_line(
            stages.last_mut().expect("stage always present"),
            &mut current_token,
            &mut pending_redirect,
        )?;

        // Every stage between pipes must carry something: `| cmd`, `cmd |`,
        // and `a | | b` are syntax errors, matching bash.
        Self::validate_stages(&stages)?;

        Ok(stages)
    }

    /// Commits the line's trailing word and rejects an operator still
    /// awaiting its filename (`echo hi >`, or `> "$EMPTY"` expanding to
    /// nothing).
    fn finish_line(
        stage: &mut ParsedCommand,
        current_token: &mut String,
        pending_redirect: &mut Option<RedirectMode>,
    ) -> ShellResult<()> {
        Self::flush_token(stage, current_token, pending_redirect);
        if pending_redirect.is_some() {
            return Err(ShellError::ParseError(
                "expected filename after redirection".to_string(),
            ));
        }
        Ok(())
    }

    /// Rejects pipelines with empty stages: a leading `|`, a trailing `|`, or
    /// nothing between two pipes. Single-command lines are always accepted.
    fn validate_stages(stages: &[ParsedCommand]) -> ShellResult<()> {
        if stages.len() > 1
            && let Some(bad) = stages.iter().position(ParsedCommand::is_empty)
        {
            let message = if bad == 0 {
                "unexpected '|'"
            } else {
                "expected command after '|'"
            };
            return Err(ShellError::ParseError(message.to_string()));
        }
        Ok(())
    }

    /// Rejects an unquoted `||` (reserved for Phase 1.4) and a pipe that
    /// would cut short a redirect's filename (`echo > | cat`).
    fn check_pipe(
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        pending_redirect: Option<RedirectMode>,
        current_token: &str,
    ) -> ShellResult<()> {
        if chars.peek() == Some(&'|') {
            chars.next();
            return Err(ShellError::ParseError(
                "'||' is reserved for a future release".to_string(),
            ));
        }
        if pending_redirect.is_some() && current_token.is_empty() {
            return Err(ShellError::ParseError(
                "expected filename after redirection".to_string(),
            ));
        }
        Ok(())
    }

    /// Handles one character inside double quotes: the closing quote,
    /// escapes (`\"`, `\\`, `\$`), and variable expansion. Returns `true`
    /// when the quote closed.
    fn double_quote_step(
        ch: char,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        current_token: &mut String,
        last_status: i32,
    ) -> bool {
        if ch == '"' {
            return true;
        }
        if ch == '\\' {
            match chars.peek() {
                Some(&next_ch) if next_ch == '"' || next_ch == '\\' || next_ch == '$' => {
                    current_token.push(chars.next().expect("peeked char"));
                }
                _ => current_token.push('\\'),
            }
            return false;
        }
        if ch == '$' {
            let var_value = Self::extract_var(chars, last_status);
            current_token.push_str(&var_value);
            return false;
        }
        current_token.push(ch);
        false
    }

    /// Commits the token being built either as a pending redirection target
    /// or as a command argument.
    fn flush_token(
        stage: &mut ParsedCommand,
        current_token: &mut String,
        pending_redirect: &mut Option<RedirectMode>,
    ) {
        if current_token.is_empty() {
            return;
        }
        if let Some(mode) = pending_redirect.take() {
            stage.redirects.push(Redirection {
                mode,
                path: current_token.clone(),
            });
        } else {
            stage.args.push(current_token.clone());
        }
        current_token.clear();
    }

    /// Handles an unquoted redirection operator. The operator terminates the
    /// current word (`echo hi>out` works), and two operators in a row without
    /// a filename (`echo > > f`) are a syntax error.
    fn start_redirect(
        stage: &mut ParsedCommand,
        current_token: &mut String,
        pending_redirect: &mut Option<RedirectMode>,
        mode: RedirectMode,
    ) -> ShellResult<()> {
        if pending_redirect.is_some() && current_token.is_empty() {
            return Err(ShellError::ParseError(
                "expected filename after redirection".to_string(),
            ));
        }
        Self::flush_token(stage, current_token, pending_redirect);
        *pending_redirect = Some(mode);
        Ok(())
    }

    fn extract_var<I>(chars: &mut std::iter::Peekable<I>, last_status: i32) -> String
    where
        I: Iterator<Item = char>,
    {
        if let Some(&'?') = chars.peek() {
            chars.next();
            return last_status.to_string();
        }

        let mut var_name = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_alphanumeric() || c == '_' {
                var_name.push(chars.next().unwrap());
            } else {
                break;
            }
        }

        if var_name.is_empty() {
            "$".to_string()
        } else {
            env::var(&var_name).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Parser, RedirectMode};
    use crate::error::ShellError;
    use std::env;

    fn parse(input: &str) -> crate::parser::ParsedCommand {
        let stages = Parser::parse(input, 0).expect("parse should succeed");
        assert_eq!(stages.len(), 1, "expected a single stage: {input:?}");
        stages.into_iter().next().expect("one stage")
    }

    fn parse_stages(input: &str) -> Vec<crate::parser::ParsedCommand> {
        Parser::parse(input, 0).expect("parse should succeed")
    }

    fn parse_err(input: &str) -> String {
        match Parser::parse(input, 0) {
            Err(ShellError::ParseError(msg)) => msg,
            other => panic!("expected parse error for {input:?}, got {other:?}"),
        }
    }

    #[test]
    fn plain_tokens() {
        let cmd = parse("echo hello world");
        assert_eq!(cmd.args, vec!["echo", "hello", "world"]);
        assert!(cmd.redirects.is_empty());
    }

    #[test]
    fn quotes_and_escapes_preserved() {
        let cmd = parse("echo 'a  b' \"c d\"");
        assert_eq!(cmd.args, vec!["echo", "a  b", "c d"]);

        let cmd = parse(r"echo 'a > b' \> \x");
        assert_eq!(cmd.args, vec!["echo", "a > b", ">", "x"]);
        assert!(cmd.redirects.is_empty());
    }

    #[test]
    fn output_trunc_spaced_and_attached() {
        let cmd = parse("echo hi > out.txt");
        assert_eq!(cmd.args, vec!["echo", "hi"]);
        assert_eq!(cmd.redirects.len(), 1);
        assert_eq!(cmd.redirects[0].mode, RedirectMode::OutputTrunc);
        assert_eq!(cmd.redirects[0].path, "out.txt");

        let cmd = parse("echo hi >out.txt");
        assert_eq!(cmd.redirects[0].path, "out.txt");

        let cmd = parse("echo hi>out.txt");
        assert_eq!(cmd.args, vec!["echo", "hi"]);
        assert_eq!(cmd.redirects[0].path, "out.txt");
    }

    #[test]
    fn append_and_input() {
        let cmd = parse("echo b >> log");
        assert_eq!(cmd.redirects[0].mode, RedirectMode::OutputAppend);
        assert_eq!(cmd.redirects[0].path, "log");

        let cmd = parse("sort < in.txt > out.txt");
        assert_eq!(cmd.args, vec!["sort"]);
        assert_eq!(cmd.redirects[0].mode, RedirectMode::Input);
        assert_eq!(cmd.redirects[0].path, "in.txt");
        assert_eq!(cmd.redirects[1].mode, RedirectMode::OutputTrunc);
        assert_eq!(cmd.redirects[1].path, "out.txt");
    }

    #[test]
    fn quoted_filename_with_spaces() {
        let cmd = parse(r#"echo hi > "my file.txt""#);
        assert_eq!(cmd.redirects[0].path, "my file.txt");
    }

    #[test]
    fn variable_expansion_in_filename() {
        // SAFETY: single-threaded test manipulating its own env var.
        unsafe {
            env::set_var("SIBSH_TEST_DIR", "/tmp/sibsh_test");
        }
        let cmd = parse("echo hi > $SIBSH_TEST_DIR/out.log");
        assert_eq!(cmd.redirects[0].path, "/tmp/sibsh_test/out.log");
    }

    #[test]
    fn missing_filename_is_error() {
        let err = Parser::parse("echo hi >", 0).unwrap_err();
        assert!(matches!(err, ShellError::ParseError(msg) if msg.contains("filename")));

        let err = Parser::parse("echo > < f", 0).unwrap_err();
        assert!(matches!(err, ShellError::ParseError(_)));
    }

    #[test]
    fn empty_expanded_filename_is_error() {
        // SAFETY: single-threaded test manipulating its own env var.
        unsafe {
            env::set_var("SIBSH_TEST_EMPTY", "");
        }
        let err = Parser::parse("> $SIBSH_TEST_EMPTY", 0).unwrap_err();
        assert!(matches!(err, ShellError::ParseError(_)));
    }

    #[test]
    fn unclosed_quote_is_error() {
        let err = Parser::parse("echo \"unclosed", 0).unwrap_err();
        assert!(matches!(err, ShellError::ParseError(msg) if msg.contains("unclosed")));
    }

    // ---- Phase 1.3: pipelines ----

    #[test]
    fn splits_two_stages() {
        let stages = parse_stages("echo hello | cat");
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].args, vec!["echo", "hello"]);
        assert_eq!(stages[1].args, vec!["cat"]);
        assert!(stages.iter().all(|s| s.redirects.is_empty()));
    }

    #[test]
    fn pipe_without_spaces_splits_words() {
        let stages = parse_stages("echo hi|wc -c");
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].args, vec!["echo", "hi"]);
        assert_eq!(stages[1].args, vec!["wc", "-c"]);
    }

    #[test]
    fn three_stage_chain() {
        let stages = parse_stages("cat f | grep x | wc -l");
        assert_eq!(stages.len(), 3);
        assert_eq!(stages[0].args, vec!["cat", "f"]);
        assert_eq!(stages[1].args, vec!["grep", "x"]);
        assert_eq!(stages[2].args, vec!["wc", "-l"]);
    }

    #[test]
    fn quoted_pipe_stays_literal_single_stage() {
        let cmd = parse("echo 'a | b'");
        assert_eq!(cmd.args, vec!["echo", "a | b"]);

        let cmd = parse("echo \"x|y\"");
        assert_eq!(cmd.args, vec!["echo", "x|y"]);

        let cmd = parse(r"echo a\|b");
        assert_eq!(cmd.args, vec!["echo", "a|b"]);
    }

    #[test]
    fn each_stage_keeps_its_own_redirections() {
        let stages = parse_stages("cat < in | sort > out | wc -c >> log");
        assert_eq!(stages.len(), 3);
        assert_eq!(stages[0].redirects[0].mode, RedirectMode::Input);
        assert_eq!(stages[0].redirects[0].path, "in");
        assert_eq!(stages[1].redirects[0].mode, RedirectMode::OutputTrunc);
        assert_eq!(stages[1].redirects[0].path, "out");
        assert_eq!(stages[2].redirects[0].mode, RedirectMode::OutputAppend);
        assert_eq!(stages[2].redirects[0].path, "log");
    }

    #[test]
    fn leading_pipe_is_syntax_error() {
        let msg = parse_err("| cat");
        assert!(msg.contains("unexpected"), "{msg}");

        let msg = parse_err("   | cat");
        assert!(msg.contains("unexpected"), "{msg}");
    }

    #[test]
    fn trailing_pipe_is_syntax_error() {
        let msg = parse_err("cat |");
        assert!(msg.contains("expected command after '|'"), "{msg}");

        let msg = parse_err("cat |   ");
        assert!(msg.contains("expected command after '|'"), "{msg}");
    }

    #[test]
    fn empty_middle_stage_is_syntax_error() {
        let msg = parse_err("a | | b");
        assert!(msg.contains("expected command after '|'"), "{msg}");
    }

    #[test]
    fn double_pipe_is_reserved_error() {
        let msg = parse_err("a || b");
        assert!(msg.contains("reserved"), "{msg}");
    }

    #[test]
    fn redirect_before_pipe_is_filename_error() {
        let msg = parse_err("echo > | cat");
        assert!(msg.contains("filename"), "{msg}");
    }

    #[test]
    fn variable_expansion_works_per_stage() {
        // SAFETY: single-threaded test manipulating its own env var.
        unsafe {
            env::set_var("SIBSH_PIPE_VAR", "zz");
        }
        let stages = parse_stages("echo $SIBSH_PIPE_VAR | cat");
        assert_eq!(stages[0].args, vec!["echo", "zz"]);
        assert_eq!(stages[1].args, vec!["cat"]);
    }

    #[test]
    fn status_expansion() {
        let stages = Parser::parse("echo code:$?", 7).expect("parse should succeed");
        assert_eq!(stages[0].args, vec!["echo", "code:7"]);
    }

    // ---- Additional edge-case coverage ----

    #[test]
    fn empty_and_whitespace_lines_parse_to_nothing() {
        for input in ["", "   ", "\t\t", " \t "] {
            let cmd = parse(input);
            assert!(cmd.args.is_empty(), "input {input:?}");
            assert!(cmd.redirects.is_empty(), "input {input:?}");
        }
    }

    #[test]
    fn bare_redirect_line_parses() {
        let cmd = parse("> out.txt");
        assert!(cmd.args.is_empty());
        assert_eq!(cmd.redirects[0].path, "out.txt");

        let cmd = parse("< in.txt");
        assert_eq!(cmd.redirects[0].mode, RedirectMode::Input);
    }

    #[test]
    fn lone_dollar_is_literal() {
        let cmd = parse("echo $");
        assert_eq!(cmd.args, vec!["echo", "$"]);

        let cmd = parse("echo $ ");
        assert_eq!(cmd.args, vec!["echo", "$"]);
    }

    #[test]
    fn unset_variable_expands_to_empty() {
        // SAFETY: single-threaded test manipulating its own env var.
        unsafe {
            env::remove_var("SIBSH_DEFINITELY_UNSET_XYZ");
        }
        let cmd = parse("echo [$SIBSH_DEFINITELY_UNSET_XYZ]");
        assert_eq!(cmd.args, vec!["echo", "[]"]);
    }

    #[test]
    fn digits_underscore_in_var_names() {
        // SAFETY: single-threaded test manipulating its own env var.
        unsafe {
            env::set_var("SIBSH_V2_x", "ok");
        }
        let cmd = parse("$SIBSH_V2_x");
        assert_eq!(cmd.args, vec!["ok"]);
    }

    #[test]
    fn variable_boundary_stops_at_punctuation() {
        // SAFETY: single-threaded test manipulating its own env var.
        unsafe {
            env::set_var("SIBSH_AB", "yes");
        }
        let cmd = parse("$SIBSH_AB/file");
        assert_eq!(cmd.args, vec!["yes/file"]);
    }

    #[test]
    fn tab_separated_tokens() {
        let cmd = parse("echo\ta\tb");
        assert_eq!(cmd.args, vec!["echo", "a", "b"]);
    }

    #[test]
    fn redirect_target_after_tab() {
        let cmd = parse("echo hi >\tf.txt");
        assert_eq!(cmd.redirects[0].path, "f.txt");
    }

    #[test]
    fn adjacent_quotes_concatenate_into_one_token() {
        let cmd = parse(r"echo a'b c'd");
        assert_eq!(cmd.args, vec!["echo", "ab cd"]);

        let cmd = parse(r#"echo pre"mid"post"#);
        assert_eq!(cmd.args, vec!["echo", "premidpost"]);
    }

    #[test]
    fn escaped_backslash_and_specials_outside_quotes() {
        let cmd = parse(r"echo a\\b");
        assert_eq!(cmd.args, vec!["echo", "a\\b"]);

        let cmd = parse(r"echo \$HOME");
        assert_eq!(cmd.args, vec!["echo", "$HOME"]);
    }

    #[test]
    fn trailing_backslash_is_dropped_not_error() {
        let cmd = parse("echo x\\");
        assert_eq!(cmd.args, vec!["echo", "x"]);
    }

    #[test]
    fn multiple_output_redirects_keep_order_last_wins_semantics() {
        let cmd = parse("echo a > f1 >> f2");
        assert_eq!(cmd.redirects.len(), 2);
        assert_eq!(
            cmd.redirects[0],
            crate::parser::Redirection {
                mode: RedirectMode::OutputTrunc,
                path: "f1".into(),
            }
        );
        assert_eq!(
            cmd.redirects[1],
            crate::parser::Redirection {
                mode: RedirectMode::OutputAppend,
                path: "f2".into(),
            }
        );
    }

    #[test]
    fn operator_between_words_without_spaces() {
        let cmd = parse("a<b>c");
        assert_eq!(cmd.args, vec!["a"]);
        assert_eq!(cmd.redirects.len(), 2);
        assert_eq!(cmd.redirects[0].path, "b");
        assert_eq!(cmd.redirects[1].path, "c");
    }

    #[test]
    fn unclosed_single_quote_is_error() {
        let err = Parser::parse("echo 'unclosed", 0).unwrap_err();
        assert!(matches!(err, ShellError::ParseError(msg) if msg.contains("unclosed")));
    }

    #[test]
    fn double_quote_escape_of_other_chars_keeps_backslash() {
        let cmd = parse(r#"echo "a\nb""#);
        assert_eq!(cmd.args, vec!["echo", "a\\nb"]);
    }

    #[test]
    fn expansion_inside_double_quotes() {
        // SAFETY: single-threaded test manipulating its own env var.
        unsafe {
            env::set_var("SIBSH_DQ", "inner");
        }
        let cmd = parse("echo \"[$SIBSH_DQ]\"");
        assert_eq!(cmd.args, vec!["echo", "[inner]"]);
    }
}
