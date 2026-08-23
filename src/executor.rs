use crate::error::{ShellError, ShellResult};
use crate::parser::{RedirectMode, Redirection};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};

/// Where a command's stdin comes from.
#[derive(Debug)]
pub enum StdinSource {
    /// The shell's own stdin.
    Inherit,
    /// An opened `< file` target.
    File(File),
    /// The read end of an inter-stage pipeline pipe.
    Pipe(io::PipeReader),
}

/// Where a command's stdout goes.
#[derive(Debug)]
pub enum StdoutSink {
    Inherit,
    /// An opened `>` / `>>` target.
    File(File),
    /// The write end of an inter-stage pipeline pipe.
    Pipe(io::PipeWriter),
}

fn redirect_error(path: &str, source: io::Error) -> ShellError {
    ShellError::Redirection {
        path: path.to_string(),
        source,
    }
}

/// Duplicate an open file for a child process without consuming it.
fn clone_file(file: &File) -> ShellResult<File> {
    file.try_clone()
        .map_err(|e| ShellError::Io(io::Error::new(e.kind(), e.to_string())))
}

/// Open file handles (or attached pipes) for the current command's
/// redirected stdin/stdout. Pipe variants are only produced by pipelines;
/// plain redirections yield `Inherit` defaults plus opened files.
pub struct RedirectHandles {
    pub stdin: StdinSource,
    pub stdout: StdoutSink,
}

impl RedirectHandles {
    /// Opens every file redirection in order (like bash: all targets are
    /// created even if a later one wins). The last redirect per stream is
    /// the one actually wired up. On failure the command must not run.
    pub fn open(redirects: &[Redirection]) -> ShellResult<Self> {
        let mut stdin = StdinSource::Inherit;
        let mut stdout = StdoutSink::Inherit;

        for redirect in redirects {
            match redirect.mode {
                RedirectMode::Input => {
                    stdin = StdinSource::File(
                        File::open(&redirect.path)
                            .map_err(|e| redirect_error(&redirect.path, e))?,
                    );
                }
                RedirectMode::OutputTrunc => {
                    stdout = StdoutSink::File(
                        File::create(&redirect.path)
                            .map_err(|e| redirect_error(&redirect.path, e))?,
                    );
                }
                RedirectMode::OutputAppend => {
                    stdout = StdoutSink::File(
                        OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&redirect.path)
                            .map_err(|e| redirect_error(&redirect.path, e))?,
                    );
                }
            }
        }

        Ok(Self { stdin, stdout })
    }

    /// Stdio wiring for spawning an external command. Pipe ends are consumed
    /// (moved into the child) so the parent never keeps a writer copy — a
    /// retained write end would stop downstream stages from ever seeing EOF.
    fn stdin_stdio(&mut self) -> ShellResult<Stdio> {
        match std::mem::replace(&mut self.stdin, StdinSource::Inherit) {
            StdinSource::Inherit => Ok(Stdio::inherit()),
            StdinSource::File(file) => Ok(Stdio::from(clone_file(&file)?)),
            StdinSource::Pipe(reader) => Ok(Stdio::from(reader)),
        }
    }

    /// See `stdin_stdio`; symmetric for stdout.
    fn stdout_stdio(&mut self) -> ShellResult<Stdio> {
        match std::mem::replace(&mut self.stdout, StdoutSink::Inherit) {
            StdoutSink::Inherit => Ok(Stdio::inherit()),
            StdoutSink::File(file) => Ok(Stdio::from(clone_file(&file)?)),
            StdoutSink::Pipe(writer) => Ok(Stdio::from(writer)),
        }
    }

    /// Output handle for a builtin: a cloned file, an owned pipe end, or the
    /// process stdout when nothing is redirected (`Stdout` is `Send` and
    /// locks internally per write, so it is safe from worker threads).
    pub fn builtin_writer(&mut self) -> ShellResult<Box<dyn Write + Send>> {
        match std::mem::replace(&mut self.stdout, StdoutSink::Inherit) {
            StdoutSink::Inherit => Ok(Box::new(io::stdout())),
            StdoutSink::File(file) => Ok(Box::new(clone_file(&file)?)),
            StdoutSink::Pipe(writer) => Ok(Box::new(writer)),
        }
    }

    /// Input handle for builtins that read stdin (`cat`). Pipe ends are
    /// handed over owned; `None` means the builtin should read the shell's
    /// own stdin.
    pub fn builtin_reader(&mut self) -> ShellResult<Option<Box<dyn Read + Send>>> {
        match std::mem::replace(&mut self.stdin, StdinSource::Inherit) {
            StdinSource::Inherit => Ok(None),
            StdinSource::File(file) => Ok(Some(Box::new(clone_file(&file)?))),
            StdinSource::Pipe(reader) => Ok(Some(Box::new(reader))),
        }
    }
}

pub struct Executor;

impl Executor {
    /// Spawns an external command with any redirected/piped stdio applied
    /// and returns the running child without waiting on it. Pipelines use
    /// this to launch every stage before reaping any of them.
    pub fn spawn(
        cmd: &str,
        args: &[String],
        redirects: &mut RedirectHandles,
    ) -> ShellResult<Child> {
        let mut command = Command::new(cmd);
        command.args(args);

        // Redirected streams come from open files or pipes; everything else
        // stays inherited so interactive tools (vim, less, top) keep working.
        command.stdin(redirects.stdin_stdio()?);
        command.stdout(redirects.stdout_stdio()?);
        command.stderr(Stdio::inherit());

        match command.spawn() {
            Ok(child) => Ok(child),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Err(ShellError::CommandNotFound(cmd.to_string()))
            }
            Err(e) => Err(ShellError::Io(e)),
        }
    }

    /// Spawns an external command and waits for completion, returning the
    /// exit status code.
    pub fn execute(
        cmd: &str,
        args: &[String],
        redirects: &mut RedirectHandles,
    ) -> ShellResult<i32> {
        let mut child = Self::spawn(cmd, args, redirects)?;
        match child.wait() {
            Ok(status) => {
                let code = status.code().unwrap_or(i32::from(!status.success()));
                Ok(code)
            }
            Err(e) => Err(ShellError::Io(e)),
        }
    }
}
