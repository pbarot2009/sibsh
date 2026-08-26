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
    // A panic while stdin is in raw mode (no echo, no line buffering) would
    // otherwise leave the user's terminal stuck after sibsh exits, since
    // unwinding runs `tty::RawGuard::drop` but the default panic hook prints
    // its message and exits before that unwind necessarily completes on
    // every platform. Chain a hook that force-restores cooked mode first,
    // then falls through to the normal panic message.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tty::restore_on_exit();
        default_hook(info);
    }));

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
