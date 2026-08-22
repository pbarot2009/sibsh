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
/// redirections, in the order they appeared.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedCommand {
    pub args: Vec<String>,
    pub redirects: Vec<Redirection>,
}

pub struct Parser;

impl Parser {
    /// Tokenizes an input line into arguments and redirections, expanding
    /// variables and stripping quotes. Operators (`>`, `>>`, `<`) are only
    /// recognized outside quotes, so `echo 'a > b'` stays a literal argument.
    pub fn parse(input: &str, last_status: i32) -> ShellResult<ParsedCommand> {
        let mut command = ParsedCommand::default();
        let mut current_token = String::new();
        let mut state = ParseState::Normal;
        let mut pending_redirect: Option<RedirectMode> = None;
        let mut chars = input.chars().peekable();

        while let Some(ch) = chars.next() {
            match state {
                ParseState::Normal => match ch {
                    ' ' | '\t' | '\r' | '\n' => {
                        Self::flush_token(
                            &mut command,
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
                    '<' => {
                        Self::start_redirect(
                            &mut command,
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
                            &mut command,
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
                    if ch == '"' {
                        state = ParseState::Normal;
                    } else if ch == '\\' {
                        if let Some(&next_ch) = chars.peek() {
                            if next_ch == '"' || next_ch == '\\' || next_ch == '$' {
                                current_token.push(chars.next().unwrap());
                            } else {
                                current_token.push('\\');
                            }
                        } else {
                            current_token.push('\\');
                        }
                    } else if ch == '$' {
                        let var_value = Self::extract_var(&mut chars, last_status);
                        current_token.push_str(&var_value);
                    } else {
                        current_token.push(ch);
                    }
                }
            }
        }

        if state != ParseState::Normal {
            return Err(ShellError::ParseError(
                "unclosed quote detected".to_string(),
            ));
        }

        // Commit a trailing word: either a redirection target (`echo hi > out`)
        // or the final argument. If an operator was still awaiting a target
        // (e.g. `echo hi >`, or `> "$EMPTY"` expanding to nothing), that is a
        // syntax error.
        Self::flush_token(&mut command, &mut current_token, &mut pending_redirect);
        if pending_redirect.is_some() {
            return Err(ShellError::ParseError(
                "expected filename after redirection".to_string(),
            ));
        }

        Ok(command)
    }

    /// Commits the token being built either as a pending redirection target
    /// or as a command argument.
    fn flush_token(
        command: &mut ParsedCommand,
        current_token: &mut String,
        pending_redirect: &mut Option<RedirectMode>,
    ) {
        if current_token.is_empty() {
            return;
        }
        if let Some(mode) = pending_redirect.take() {
            command.redirects.push(Redirection {
                mode,
                path: current_token.clone(),
            });
        } else {
            command.args.push(current_token.clone());
        }
        current_token.clear();
    }

    /// Handles an unquoted redirection operator. The operator terminates the
    /// current word (`echo hi>out` works), and two operators in a row without
    /// a filename (`echo > > f`) are a syntax error.
    fn start_redirect(
        command: &mut ParsedCommand,
        current_token: &mut String,
        pending_redirect: &mut Option<RedirectMode>,
        mode: RedirectMode,
    ) -> ShellResult<()> {
        if pending_redirect.is_some() && current_token.is_empty() {
            return Err(ShellError::ParseError(
                "expected filename after redirection".to_string(),
            ));
        }
        Self::flush_token(command, current_token, pending_redirect);
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
    use super::{RedirectMode, Parser};
    use crate::error::ShellError;
    use std::env;

    fn parse(input: &str) -> crate::parser::ParsedCommand {
        Parser::parse(input, 0).expect("parse should succeed")
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

    #[test]
    fn status_expansion() {
        let cmd = Parser::parse("echo code:$?", 7).expect("parse should succeed");
        assert_eq!(cmd.args, vec!["echo", "code:7"]);
    }
}
