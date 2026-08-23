mod builtins;
mod completion;
mod config;
mod error;
mod executor;
mod parser;
mod prompt;
mod shell;

use shell::ShellState;
use std::process;

fn main() {
    // Load ~/.sibsh/sibsh.toml (if present), then run its imports
    // (bashrc/zshrc style) before the first prompt.
    let config = config::Config::load();
    let mut shell = ShellState::with_config(config);
    shell.run_imports();

    let exit_code = shell.run_repl();
    process::exit(exit_code);
}
