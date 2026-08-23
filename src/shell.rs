use crate::builtins::Builtins;
use crate::completion;
use crate::config::{self, Config};
use crate::error::{ShellError, ShellResult};
use crate::executor::{Executor, RedirectHandles};
use crate::parser::Parser;
use crate::prompt::Prompt;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

pub struct ShellState {
    pub history: Vec<String>,
    pub last_status: i32,
    pub old_pwd: Option<PathBuf>,
    pub running: bool,
    /// Loaded from `~/.sibsh/sibsh.toml`.
    pub config: Config,
    /// Alias definitions, from config plus runtime `alias` commands.
    pub aliases: Vec<(String, String)>,
}

impl ShellState {
    pub fn with_config(config: Config) -> Self {
        let aliases = config.aliases.clone();
        Self {
            history: Vec::new(),
            last_status: 0,
            old_pwd: None,
            running: true,
            config,
            aliases,
        }
    }

    /// Executes every file listed under `imports` in the config. Lines that
    /// sibsh cannot parse or run (rc files contain plenty of bash-only
    /// syntax) are skipped silently so a shared `.bashrc` still works for
    /// the parts it supports: `export`, `alias`, and plain command lines.
    pub fn run_imports(&mut self) {
        let imports = self.config.imports.clone();
        for import in imports {
            let path = config::expand_tilde(&import);
            match fs::read_to_string(&path) {
                Ok(text) => {
                    for line in text.lines().map(str::trim) {
                        if line.is_empty() || line.starts_with('#') {
                            continue;
                        }
                        // Errors are intentionally ignored during imports.
                        let _ = self.dispatch_line(line);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => eprintln!("sibsh: import {import}: {e}"),
            }
        }
    }

    /// Primary interactive REPL loop.
    pub fn run_repl(&mut self) -> i32 {
        while self.running {
            // 1. Render Prompt
            let prompt = Prompt::render_with(self.config.prompt.as_deref(), self.last_status);
            print!("{prompt}");
            if let Err(e) = io::stdout().flush() {
                eprintln!("sibsh: failed to flush stdout: {e}");
                break;
            }

            // 2. Read user input with tab completion and history navigation.
            let line = match completion::read_line(&prompt, &self.history, &self.aliases) {
                Ok(Some(line)) => line,
                // EOF reached (Ctrl+D on an empty line).
                Ok(None) => {
                    println!("\nexit");
                    break;
                }
                Err(e) => {
                    eprintln!("sibsh: read error: {e}");
                    break;
                }
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // 3. Record to history, honoring history_limit from config.
            self.history.push(trimmed.to_string());
            if let Some(limit) = self.config.history_limit.filter(|limit| *limit > 0)
                && self.history.len() > limit
            {
                let excess = self.history.len() - limit;
                self.history.drain(..excess);
            }

            // 4. Dispatch line
            match self.dispatch_line(trimmed) {
                Ok(code) => self.last_status = code,
                Err(ShellError::Exit(code)) => return code,
                Err(err) => {
                    eprintln!("{err}");
                    self.last_status = 1;
                }
            }
        }

        self.last_status
    }

    pub fn dispatch_line(&mut self, line: &str) -> ShellResult<i32> {
        let mut command = Parser::parse(line, self.last_status)?;
        if command.args.is_empty() {
            return Ok(0);
        }

        // Expand an alias in the first word once (no recursion). The alias
        // value is split on whitespace, like other basic shells do.
        if let Some(first) = command.args.first().cloned()
            && let Some((_, expansion)) = self.aliases.iter().find(|(name, _)| *name == first)
        {
            let mut expanded: Vec<String> =
                expansion.split_whitespace().map(String::from).collect();
            expanded.extend(command.args.iter().skip(1).cloned());
            command.args = expanded;
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
