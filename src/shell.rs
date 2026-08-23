use crate::builtins::Builtins;
use crate::completion;
use crate::config::{self, Config};
use crate::error::{ShellError, ShellResult};
use crate::executor::{Executor, RedirectHandles};
use crate::parser::{ParsedCommand, Parser};
use crate::pipeline;
use crate::prompt::{self, IconMode, Prompt, PromptCtx};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Instant;

/// Snapshot-able shell state. Pipelines clone this for builtin worker
/// threads; mutations inside a pipeline therefore stay local to the
/// pipeline, matching bash's subshell semantics for pipeline members.
#[derive(Clone)]
pub struct ShellState {
    pub history: Vec<String>,
    pub last_status: i32,
    /// Wall-clock time of the most recent command; feeds the prompt timer.
    pub last_duration: Option<std::time::Duration>,
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
            last_duration: None,
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
    ///
    /// Prompt painting belongs entirely to the line reader (`completion::read_line`),
    /// matching how zsh's ZLE and fish work: the editor draws the prompt and the
    /// buffer together so redraws never duplicate the prompt or lose column 0.
    pub fn run_repl(&mut self) -> i32 {
        while self.running {
            // 1. Render the prompt text. Like zsh's ZLE and fish, the line
            //    reader is the only place that paints it on screen: printing
            //    here as well would show two prompts on one line.
            let prompt = {
                let ctx = PromptCtx {
                    last_status: self.last_status,
                    last_duration: self.last_duration,
                    icons: IconMode::from_config(self.config.icons.as_deref()),
                    git_enabled: self.config.git_status,
                    root: prompt::is_root(),
                };
                Prompt::render_auto(self.config.prompt.as_deref(), &ctx)
            };

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

            // 4. Dispatch line and record how long it took for the timer.
            let started = Instant::now();
            match self.dispatch_line(trimmed) {
                Ok(code) => self.last_status = code,
                Err(ShellError::Exit(code)) => return code,
                Err(err) => {
                    eprintln!("{err}");
                    self.last_status = 1;
                }
            }
            self.last_duration = Some(started.elapsed());
        }

        self.last_status
    }

    pub fn dispatch_line(&mut self, line: &str) -> ShellResult<i32> {
        let stages = Parser::parse(line, self.last_status)?;

        if stages.len() > 1 {
            return pipeline::run(self, &stages);
        }

        let mut command: ParsedCommand = stages.into_iter().next().expect("one stage");
        if command.args.is_empty() {
            return Ok(0);
        }

        self.expand_first_alias(&mut command);

        // Open all redirection targets up front; on failure the command must
        // not run and the error propagates to the REPL (status 1).
        let mut handles = RedirectHandles::open(&command.redirects)?;

        let cmd = command.args[0].clone();
        let result = if Builtins::is_builtin(&cmd) {
            Builtins::execute(self, &command.args, &mut handles)
        } else {
            Executor::execute(&cmd, &command.args[1..], &mut handles)
        };

        // Make sure builtin output lands before the next prompt renders.
        io::stdout().flush()?;
        result
    }

    /// Expands an alias in the first word of a parsed stage, once, without
    /// recursion. The alias value is split on whitespace like in other basic
    /// shells. Used by single-command dispatch and by every pipeline stage.
    pub fn expand_first_alias(&self, stage: &mut ParsedCommand) {
        if let Some(first) = stage.args.first().cloned()
            && let Some((_, expansion)) = self.aliases.iter().find(|(name, _)| *name == first)
        {
            let mut expanded: Vec<String> =
                expansion.split_whitespace().map(String::from).collect();
            expanded.extend(stage.args.iter().skip(1).cloned());
            stage.args = expanded;
        }
    }
}
