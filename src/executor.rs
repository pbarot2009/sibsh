use crate::error::{ShellError, ShellResult};
use std::process::Command;

pub struct Executor;

impl Executor {
    /// Spawns an external command, waits for completion, and returns the exit status code.
    pub fn execute(cmd: &str, args: &[String]) -> ShellResult<i32> {
        let mut command = Command::new(cmd);
        command.args(args);

        // Inherit stdin/stdout/stderr for interactive tools (vim, less, top, etc.)
        command.stdin(std::process::Stdio::inherit());
        command.stdout(std::process::Stdio::inherit());
        command.stderr(std::process::Stdio::inherit());

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
