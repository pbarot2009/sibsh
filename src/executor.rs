use crate::error::{ShellError, ShellResult};
use crate::parser::{RedirectMode, Redirection};
use std::fs::{File, OpenOptions};
use std::io;
use std::process::{Command, Stdio};

/// Open file handles for the current command's redirected stdin/stdout.
pub struct RedirectHandles {
    pub stdin: Option<File>,
    pub stdout: Option<File>,
}

fn redirect_error(path: &str, source: io::Error) -> ShellError {
    ShellError::Redirection {
        path: path.to_string(),
        source,
    }
}

impl RedirectHandles {
    /// Opens every redirection in order (like bash: all targets are created
    /// even if a later one wins). The last redirect per stream is the one
    /// actually wired up. On failure the command must not run.
    pub fn open(redirects: &[Redirection]) -> ShellResult<Self> {
        let mut stdin: Option<File> = None;
        let mut stdout: Option<File> = None;

        for redirect in redirects {
            match redirect.mode {
                RedirectMode::Input => {
                    let file = File::open(&redirect.path)
                        .map_err(|e| redirect_error(&redirect.path, e))?;
                    stdin = Some(file);
                }
                RedirectMode::OutputTrunc => {
                    let file = File::create(&redirect.path)
                        .map_err(|e| redirect_error(&redirect.path, e))?;
                    stdout = Some(file);
                }
                RedirectMode::OutputAppend => {
                    let file = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&redirect.path)
                        .map_err(|e| redirect_error(&redirect.path, e))?;
                    stdout = Some(file);
                }
            }
        }

        Ok(Self { stdin, stdout })
    }

    fn stdin_stdio(&self) -> ShellResult<Stdio> {
        match &self.stdin {
            Some(file) => {
                Ok(Stdio::from(file.try_clone().map_err(|e| {
                    ShellError::Io(io::Error::new(e.kind(), e.to_string()))
                })?))
            }
            None => Ok(Stdio::inherit()),
        }
    }

    fn stdout_stdio(&self) -> ShellResult<Stdio> {
        match &self.stdout {
            Some(file) => {
                Ok(Stdio::from(file.try_clone().map_err(|e| {
                    ShellError::Io(io::Error::new(e.kind(), e.to_string()))
                })?))
            }
            None => Ok(Stdio::inherit()),
        }
    }
}

pub struct Executor;

impl Executor {
    /// Spawns an external command with any redirected stdio applied, waits
    /// for completion, and returns the exit status code.
    pub fn execute(cmd: &str, args: &[String], redirects: &RedirectHandles) -> ShellResult<i32> {
        let mut command = Command::new(cmd);
        command.args(args);

        // Redirected streams come from open files; everything else stays
        // inherited so interactive tools (vim, less, top) keep working.
        command.stdin(redirects.stdin_stdio()?);
        command.stdout(redirects.stdout_stdio()?);
        command.stderr(Stdio::inherit());

        match command.status() {
            Ok(status) => {
                let code = status.code().unwrap_or(i32::from(!status.success()));
                Ok(code)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(ShellError::CommandNotFound(cmd.to_string()))
            }
            Err(e) => Err(ShellError::Io(e)),
        }
    }
}
