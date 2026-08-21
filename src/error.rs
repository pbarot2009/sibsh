use std::fmt;
use std::io;

#[derive(Debug)]
pub enum ShellError {
    Io(io::Error),
    CommandNotFound(String),
    BuiltinError(String),
    ParseError(String),
    Exit(i32),
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShellError::Io(err) => write!(f, "sibsh: I/O error: {err}"),
            ShellError::CommandNotFound(cmd) => write!(f, "sibsh: command not found: {cmd}"),
            ShellError::BuiltinError(msg) => write!(f, "sibsh: {msg}"),
            ShellError::ParseError(msg) => write!(f, "sibsh: syntax error: {msg}"),
            ShellError::Exit(code) => write!(f, "exit with status {code}"),
        }
    }
}

impl std::error::Error for ShellError {}

impl From<io::Error> for ShellError {
    fn from(err: io::Error) -> Self {
        ShellError::Io(err)
    }
}

pub type ShellResult<T> = Result<T, ShellError>;
