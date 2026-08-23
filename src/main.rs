mod builtins;
mod completion;
mod config;
mod error;
mod executor;
mod parser;
mod pipeline;
mod prompt;
mod shell;
mod tty;

use shell::ShellState;
use std::process;

fn main() {
    // Create a commented template config on first run (no-op if it exists),
    // then load ~/.sibsh/sibsh.toml and run its imports (bashrc/zshrc style)
    // before the first prompt.
    config::Config::ensure_default();
    let config = config::Config::load();
    let mut shell = ShellState::with_config(config);
    shell.run_imports();

    let exit_code = shell.run_repl();
    process::exit(exit_code);
}
