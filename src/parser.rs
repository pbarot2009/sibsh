use crate::error::{ShellError, ShellResult};
use std::env;

#[derive(Debug, PartialEq, Eq)]
enum ParseState {
    Normal,
    InSingleQuote,
    InDoubleQuote,
}

pub struct Parser;

impl Parser {
    /// Tokenizes an input line into arguments, expanding variables and stripping quotes.
    pub fn parse(input: &str, last_status: i32) -> ShellResult<Vec<String>> {
        let mut tokens = Vec::new();
        let mut current_token = String::new();
        let mut state = ParseState::Normal;
        let mut chars = input.chars().peekable();

        while let Some(ch) = chars.next() {
            match state {
                ParseState::Normal => match ch {
                    ' ' | '\t' | '\r' | '\n' => {
                        if !current_token.is_empty() {
                            tokens.push(current_token);
                            current_token = String::new();
                        }
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
                ParseState::InDoubleQuote => match ch {
                    '"' => {
                        state = ParseState::Normal;
                    }
                    '\\' => {
                        if let Some(&next_ch) = chars.peek() {
                            if next_ch == '"' || next_ch == '\\' || next_ch == '$' {
                                current_token.push(chars.next().unwrap());
                            } else {
                                current_token.push('\\');
                            }
                        } else {
                            current_token.push('\\');
                        }
                    }
                    '$' => {
                        let var_value = Self::extract_var(&mut chars, last_status);
                        current_token.push_str(&var_value);
                    }
                    _ => {
                        current_token.push(ch);
                    }
                },
            }
        }

        if state != ParseState::Normal {
            return Err(ShellError::ParseError(
                "unclosed quote detected".to_string(),
            ));
        }

        if !current_token.is_empty() {
            tokens.push(current_token);
        }

        Ok(tokens)
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
