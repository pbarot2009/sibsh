mod builtins;
mod error;
mod executor;
mod parser;
mod prompt;
mod shell;

use shell::ShellState;
use std::process;

fn main() {
    let mut shell = ShellState::new();
    let exit_code = shell.run_repl();
    process::exit(exit_code);
}
