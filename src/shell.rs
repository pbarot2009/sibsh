use crate::builtins::Builtins;
use crate::error::{ShellError, ShellResult};
use crate::executor::Executor;
use crate::parser::Parser;
use crate::prompt::Prompt;
use std::io::{self, BufRead, Write};
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
        let mut reader = stdin.lock();

        while self.running {
            // 1. Render Prompt
            let prompt = Prompt::render(self.last_status);
            print!("{prompt}");
            if let Err(e) = io::stdout().flush() {
                eprintln!("sibsh: failed to flush stdout: {e}");
                break;
            }

            // 2. Read user input
            let mut line = String::new();
            match reader.read_line(&mut line) {
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
        let tokens = Parser::parse(line, self.last_status)?;
        if tokens.is_empty() {
            return Ok(0);
        }

        let cmd = &tokens[0];
        if Builtins::is_builtin(cmd) {
            Builtins::execute(self, &tokens)
        } else {
            Executor::execute(cmd, &tokens[1..])
        }
    }
}
