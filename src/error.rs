use std::fmt;
use std::io;

#[derive(Debug)]
pub enum ShellError {
    Io(io::Error),
    CommandNotFound(String),
    BuiltinError(String),
    ParseError(String),
    Redirection { path: String, source: io::Error },
    Exit(i32),
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShellError::Io(err) => write!(f, "sibsh: I/O error: {err}"),
            ShellError::CommandNotFound(cmd) => write!(f, "sibsh: command not found: {cmd}"),
            ShellError::BuiltinError(msg) => write!(f, "sibsh: {msg}"),
            ShellError::ParseError(msg) => write!(f, "sibsh: syntax error: {msg}"),
            ShellError::Redirection { path, source } => write!(f, "sibsh: {path}: {source}"),
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

impl From<ShellError> for io::Error {
    fn from(err: ShellError) -> Self {
        match err {
            ShellError::Io(io_err) => io_err,
            ShellError::Redirection { source, .. } => source,
            other => io::Error::other(other.to_string()),
        }
    }
}

pub type ShellResult<T> = Result<T, ShellError>;
