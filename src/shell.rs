use crate::builtins::Builtins;
use crate::error::{ShellError, ShellResult};
use crate::executor::{Executor, RedirectHandles};
use crate::parser::Parser;
use crate::prompt::Prompt;
use std::io::{self, Write};
use std::path::PathBuf;

pub struct ShellState {
    pub history: Vec<String>,
    pub last_status: i32,
    pub old_pwd: Option<PathBuf>,
    pub running: bool,
}

impl ShellState {
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            last_status: 0,
            old_pwd: None,
            running: true,
        }
    }

    /// Primary Interactive REPL Loop
    pub fn run_repl(&mut self) -> i32 {
        let stdin = io::stdin();

        while self.running {
            // 1. Render Prompt
            let prompt = Prompt::render(self.last_status);
            print!("{prompt}");
            if let Err(e) = io::stdout().flush() {
                eprintln!("sibsh: failed to flush stdout: {e}");
                break;
            }

            // 2. Read user input (per-call lock so builtins like `cat` can
            //    read stdin themselves without contending on the lock)
            let mut line = String::new();
            match stdin.read_line(&mut line) {
                Ok(0) => {
                    // EOF reached (Ctrl+D)
                    println!("\nexit");
                    break;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    // 3. Record to history
                    self.history.push(trimmed.to_string());

                    // 4. Dispatch line
                    match self.dispatch_line(trimmed) {
                        Ok(code) => {
                            self.last_status = code;
                        }
                        Err(ShellError::Exit(code)) => {
                            return code;
                        }
                        Err(err) => {
                            eprintln!("{err}");
                            self.last_status = 1;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("sibsh: read error: {e}");
                    break;
                }
            }
        }

        self.last_status
    }

    pub fn dispatch_line(&mut self, line: &str) -> ShellResult<i32> {
        let command = Parser::parse(line, self.last_status)?;
        if command.args.is_empty() {
            return Ok(0);
        }

        // Open all redirection targets up front; on failure the command must
        // not run and the error propagates to the REPL (status 1).
        let handles = RedirectHandles::open(&command.redirects)?;

        let cmd = &command.args[0];
        let result = if Builtins::is_builtin(cmd) {
            Builtins::execute(self, &command.args, &handles)
        } else {
            Executor::execute(cmd, &command.args[1..], &handles)
        };

        // Make sure builtin output lands before the next prompt renders.
        io::stdout().flush()?;
        result
    }
}
