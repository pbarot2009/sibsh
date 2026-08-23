use crate::error::{ShellError, ShellResult};
use crate::executor::RedirectHandles;
use crate::shell::ShellState;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub struct Builtins;

impl Builtins {
    /// All builtin names, used by `help`, completion, and `is_builtin`.
    pub const NAMES: [&str; 19] = [
        "cd", "pwd", "echo", "exit", "help", "clear", "type", "which", "env", "export", "setenv",
        "unset", "unsetenv", "history", "touch", "cat", "true", "false", "alias",
    ];

    pub fn is_builtin(cmd: &str) -> bool {
        Self::NAMES.contains(&cmd) || cmd == "unalias"
    }

    pub fn execute(
        state: &mut ShellState,
        args: &[String],
        redirects: &mut RedirectHandles,
    ) -> ShellResult<i32> {
        let out = redirects.builtin_writer()?;
        let mut input = redirects.builtin_reader()?;
        Self::dispatch(state, args, &mut input, out)
    }

    /// Core builtin dispatcher over explicit I/O streams. The same code path
    /// serves interactive use, file redirection, and pipeline worker threads:
    /// `input` is `Some` only when a file or pipe feeds stdin, and `out` is
    /// wherever output must land (stdout lock, file, or pipe).
    pub fn dispatch(
        state: &mut ShellState,
        args: &[String],
        input: &mut Option<Box<dyn Read + Send>>,
        mut out: Box<dyn Write + Send>,
    ) -> ShellResult<i32> {
        if args.is_empty() {
            return Ok(0);
        }

        let cmd = args[0].as_str();
        let cmd_args = &args[1..];

        let result = match cmd {
            "cd" => Ok(Self::builtin_cd(state, cmd_args, &mut out)),
            "pwd" => Ok(Self::builtin_pwd(&mut out)),
            "echo" => Self::builtin_echo(&mut out, cmd_args),
            "exit" => Self::builtin_exit(state, cmd_args),
            "help" => Ok(Self::builtin_help(&mut out)),
            "clear" => Self::builtin_clear(&mut out),
            "type" => Ok(Self::builtin_type(&mut out, cmd_args)),
            "which" => Ok(Self::builtin_which(&mut out, cmd_args)),
            "env" => Ok(Self::builtin_env(&mut out)),
            "export" | "setenv" => Ok(Self::builtin_export(cmd_args)),
            "unset" | "unsetenv" => Ok(Self::builtin_unset(cmd_args)),
            "history" => Ok(Self::builtin_history(&mut out, state)),
            "touch" => Ok(Self::builtin_touch(cmd_args)),
            "cat" => Self::builtin_cat(input, &mut out, cmd_args),
            "true" => Ok(0),
            "false" => Ok(1),
            "alias" | "unalias" => Ok(Self::builtin_alias(state, cmd, cmd_args, &mut out)),
            _ => Err(ShellError::BuiltinError(format!("unknown builtin: {cmd}"))),
        };

        out.flush()?;
        result
    }

    /// `alias` lists all aliases; `alias name=value` defines one at runtime;
    /// `unalias name` removes one.
    fn builtin_alias(
        state: &mut ShellState,
        cmd: &str,
        args: &[String],
        out: &mut dyn Write,
    ) -> i32 {
        if cmd == "unalias" {
            if args.is_empty() {
                eprintln!("sibsh: unalias: missing operand");
                return 1;
            }
            for name in args {
                if !state.aliases.iter().any(|(key, _)| key == name) {
                    eprintln!("sibsh: unalias: {name}: not found");
                    return 1;
                }
                state.aliases.retain(|(key, _)| key != name);
            }
            return 0;
        }

        if args.is_empty() {
            for (name, value) in &state.aliases {
                let _ = writeln!(out, "{name}='{value}'");
            }
            return 0;
        }

        let mut status = 0;
        for arg in args {
            match arg.split_once('=') {
                Some((name, value)) if !name.is_empty() => {
                    let value = value.trim_matches('\'').trim_matches('"');
                    state.aliases.retain(|(key, _)| key != name);
                    state.aliases.push((name.to_string(), value.to_string()));
                }
                _ => {
                    eprintln!("sibsh: alias: `{arg}`: not a valid definition (use name=value)");
                    status = 1;
                }
            }
        }
        status
    }

    fn builtin_cd(state: &mut ShellState, args: &[String], out: &mut dyn Write) -> i32 {
        let target_path = if args.is_empty() || args[0] == "~" {
            env::var("HOME").unwrap_or_else(|_| "/".to_string())
        } else if args[0] == "-" {
            if let Some(prev) = &state.old_pwd {
                let path_str = prev.to_string_lossy().to_string();
                let _ = writeln!(out, "{path_str}");
                path_str
            } else {
                eprintln!("sibsh: cd: OLDPWD not set");
                return 1;
            }
        } else {
            args[0].clone()
        };

        let current = env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let target = Path::new(&target_path);

        if let Err(e) = env::set_current_dir(target) {
            eprintln!("sibsh: cd: {target_path}: {e}");
            return 1;
        }

        state.old_pwd = Some(current);
        0
    }

    fn builtin_pwd(out: &mut dyn Write) -> i32 {
        match env::current_dir() {
            Ok(path) => {
                let _ = writeln!(out, "{}", path.display());
                0
            }
            Err(e) => {
                eprintln!("sibsh: pwd: {e}");
                1
            }
        }
    }

    fn builtin_echo(out: &mut dyn Write, args: &[String]) -> ShellResult<i32> {
        let mut no_newline = false;
        let mut start_idx = 0;

        if !args.is_empty() && args[0] == "-n" {
            no_newline = true;
            start_idx = 1;
        }

        let output = args[start_idx..].join(" ");
        if no_newline {
            write!(out, "{output}")?;
        } else {
            writeln!(out, "{output}")?;
        }
        Ok(0)
    }

    fn builtin_exit(state: &mut ShellState, args: &[String]) -> ShellResult<i32> {
        let code = if let Some(first) = args.first() {
            first.parse::<i32>().unwrap_or(0)
        } else {
            state.last_status
        };
        state.running = false;
        Err(ShellError::Exit(code))
    }

    fn builtin_help(out: &mut dyn Write) -> i32 {
        let _ = writeln!(out, "\x1b[1msibsh - Something Is Better Shell\x1b[0m");
        let _ = writeln!(out, "Type program names and arguments, then hit enter.");
        let _ = writeln!(
            out,
            "Tab completes commands and file paths; Up/Down browse history."
        );
        let _ = writeln!(
            out,
            "Redirection: cmd > file (create), cmd >> file (append), cmd < file (stdin)"
        );
        let _ = writeln!(
            out,
            "Pipelines: cmd1 | cmd2 | cmd3 (stages run concurrently)"
        );
        let _ = writeln!(
            out,
            "Config: ~/.sibsh/sibsh.toml supports prompt, aliases, and bashrc/zshrc imports\n"
        );
        let _ = writeln!(out, "Built-in Commands:");
        let _ = writeln!(
            out,
            "  cd <dir>            Change working directory (supports ~, -)"
        );
        let _ = writeln!(out, "  pwd                 Print current working directory");
        let _ = writeln!(out, "  echo [-n] <args>    Print text to stdout");
        let _ = writeln!(out, "  exit [code]         Exit the shell");
        let _ = writeln!(out, "  clear               Clear the terminal screen");
        let _ = writeln!(
            out,
            "  type <cmd>          Describe how command would be interpreted"
        );
        let _ = writeln!(out, "  which <cmd>         Locate a program in $PATH");
        let _ = writeln!(
            out,
            "  env                 Display all environment variables"
        );
        let _ = writeln!(
            out,
            "  export KEY=VAL      Set an environment variable (export KEY marks it empty)"
        );
        let _ = writeln!(out, "  unset KEY           Remove an environment variable");
        let _ = writeln!(out, "  history             Show command history");
        let _ = writeln!(
            out,
            "  touch <file...>     Create files or update their modification time"
        );
        let _ = writeln!(
            out,
            "  cat <file...>       Concatenate and print file contents"
        );
        let _ = writeln!(
            out,
            "  alias               List aliases; alias name='value' defines one"
        );
        let _ = writeln!(out, "  unalias <name>      Remove an alias");
        let _ = writeln!(out, "  true / false        Return exit status 0 or 1");
        let _ = writeln!(out, "  help                Show this help message");
        0
    }

    fn builtin_clear(out: &mut dyn Write) -> ShellResult<i32> {
        write!(out, "\x1b[2J\x1b[H")?;
        out.flush()?;
        Ok(0)
    }

    fn builtin_type(out: &mut dyn Write, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("sibsh: type: missing operand");
            return 1;
        }

        let mut status = 0;
        for target in args {
            if Self::is_builtin(target) {
                let _ = writeln!(out, "{target} is a shell builtin");
            } else if let Some(path) = Self::resolve_in_path(target) {
                let _ = writeln!(out, "{target} is {}", path.display());
            } else {
                eprintln!("sibsh: type: {target}: not found");
                status = 1;
            }
        }
        status
    }

    fn builtin_which(out: &mut dyn Write, args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("sibsh: which: missing operand");
            return 1;
        }

        let mut status = 0;
        for target in args {
            if let Some(path) = Self::resolve_in_path(target) {
                let _ = writeln!(out, "{}", path.display());
            } else {
                status = 1;
            }
        }
        status
    }

    fn builtin_env(out: &mut dyn Write) -> i32 {
        for (key, val) in env::vars() {
            let _ = writeln!(out, "{key}={val}");
        }
        0
    }

    fn builtin_export(args: &[String]) -> i32 {
        if args.is_empty() {
            let mut out = io::stdout().lock();
            return Self::builtin_env(&mut out);
        }

        for arg in args {
            if let Some((key, val)) = arg.split_once('=') {
                if key.is_empty() {
                    eprintln!("sibsh: export: `{arg}`: not a valid identifier");
                    return 1;
                }
                // SAFETY: Modifying environment variables in a single-threaded REPL is safe.
                unsafe {
                    env::set_var(key, val);
                }
            } else if env::var(arg).is_err() {
                // SAFETY: Modifying environment variables in a single-threaded REPL is safe.
                unsafe {
                    env::set_var(arg, "");
                }
            }
        }
        0
    }

    fn builtin_unset(args: &[String]) -> i32 {
        for arg in args {
            // SAFETY: Modifying environment variables in a single-threaded REPL is safe.
            unsafe {
                env::remove_var(arg);
            }
        }
        0
    }

    fn builtin_history(out: &mut dyn Write, state: &ShellState) -> i32 {
        for (idx, cmd) in state.history.iter().enumerate() {
            let _ = writeln!(out, "{:5}  {cmd}", idx + 1);
        }
        0
    }

    fn builtin_touch(args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("sibsh: touch: missing file operand");
            return 1;
        }

        let mut status = 0;
        let now = std::time::SystemTime::now();
        for path_str in args {
            // Create the file when missing, then stamp it with the current
            // time like real touch does.
            let res = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(path_str)
                .and_then(|file| file.set_modified(now));

            if let Err(e) = res {
                eprintln!("sibsh: touch: cannot touch '{path_str}': {e}");
                status = 1;
            }
        }
        status
    }

    /// Byte-safe `cat`: copies raw bytes so binary files survive intact.
    /// With no operands it echoes stdin — a redirected file or pipe when one
    /// is attached, otherwise the shell's own stdin.
    fn builtin_cat(
        input: &mut Option<Box<dyn Read + Send>>,
        out: &mut dyn Write,
        args: &[String],
    ) -> ShellResult<i32> {
        if args.is_empty() {
            match input.as_deref_mut() {
                Some(reader) => {
                    io::copy(reader, out)?;
                }
                None => {
                    io::copy(&mut io::stdin(), out)?;
                }
            }
            return Ok(0);
        }

        let mut status = 0;
        for path_str in args {
            match fs::File::open(path_str) {
                Ok(mut file) => {
                    if let Err(e) = io::copy(&mut file, out) {
                        eprintln!("sibsh: cat: {path_str}: {e}");
                        status = 1;
                    }
                }
                Err(e) => {
                    eprintln!("sibsh: cat: {path_str}: {e}");
                    status = 1;
                }
            }
        }
        Ok(status)
    }

    pub fn resolve_in_path(cmd: &str) -> Option<PathBuf> {
        if cmd.contains('/') {
            let path = PathBuf::from(cmd);
            if path.is_file() {
                return Some(path);
            }
            return None;
        }

        if let Ok(path_var) = env::var("PATH") {
            for dir in env::split_paths(&path_var) {
                let candidate = dir.join(cmd);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
    }
}
