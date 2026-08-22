use crate::error::{ShellError, ShellResult};
use crate::executor::RedirectHandles;
use crate::shell::ShellState;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub struct Builtins;

impl Builtins {
    pub fn is_builtin(cmd: &str) -> bool {
        matches!(
            cmd,
            "cd" | "pwd"
                | "echo"
                | "exit"
                | "help"
                | "clear"
                | "type"
                | "which"
                | "env"
                | "export"
                | "setenv"
                | "unset"
                | "unsetenv"
                | "history"
                | "touch"
                | "cat"
                | "true"
                | "false"
        )
    }

    pub fn execute(
        state: &mut ShellState,
        args: &[String],
        redirects: &RedirectHandles,
    ) -> ShellResult<i32> {
        if args.is_empty() {
            return Ok(0);
        }

        let cmd = args[0].as_str();
        let cmd_args = &args[1..];

        // Builtins write through an explicit handle so `echo hi > f.txt`
        // redirects without touching global stdout state.
        let mut out: Box<dyn Write> = match &redirects.stdout {
            Some(file) => Box::new(file.try_clone()?),
            None => Box::new(io::stdout().lock()),
        };

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
            "cat" => Self::builtin_cat(redirects.stdin.as_ref(), &mut out, cmd_args),
            "true" => Ok(0),
            "false" => Ok(1),
            _ => Err(ShellError::BuiltinError(format!("unknown builtin: {cmd}"))),
        };

        out.flush()?;
        result
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
        let _ = writeln!(out, "\x1b[1msibsh - Something Is Better Shell (Phase 1.2)\x1b[0m");
        let _ = writeln!(out, "Type program names and arguments, then hit enter.");
        let _ = writeln!(out, "Redirection: cmd > file (create), cmd >> file (append), cmd < file (stdin)\n");
        let _ = writeln!(out, "Built-in Commands:");
        let _ = writeln!(out, "  cd <dir>            Change working directory (supports ~, -)");
        let _ = writeln!(out, "  pwd                 Print current working directory");
        let _ = writeln!(out, "  echo [-n] <args>    Print text to stdout");
        let _ = writeln!(out, "  exit [code]         Exit the shell");
        let _ = writeln!(out, "  clear               Clear the terminal screen");
        let _ = writeln!(out, "  type <cmd>          Describe how command would be interpreted");
        let _ = writeln!(out, "  which <cmd>         Locate a program in $PATH");
        let _ = writeln!(out, "  env                 Display all environment variables");
        let _ = writeln!(out, "  export KEY=VAL      Set an environment variable");
        let _ = writeln!(out, "  unset KEY           Remove an environment variable");
        let _ = writeln!(out, "  history             Show command history");
        let _ = writeln!(out, "  touch <file...>     Create or update file timestamp");
        let _ = writeln!(out, "  cat <file...>       Concatenate and print file contents");
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
        for path_str in args {
            let res = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(path_str);

            if let Err(e) = res {
                eprintln!("sibsh: touch: cannot touch '{path_str}': {e}");
                status = 1;
            }
        }
        status
    }

    /// Byte-safe `cat`: copies raw bytes so binary files survive intact.
    /// With no operands it echoes stdin (or the redirected input file).
    fn builtin_cat(
        stdin_file: Option<&fs::File>,
        out: &mut dyn Write,
        args: &[String],
    ) -> ShellResult<i32> {
        let mut input: Box<dyn Read> = match stdin_file {
            Some(file) => Box::new(file.try_clone()?),
            None => Box::new(io::stdin()),
        };
        if args.is_empty() {
            io::copy(&mut input, out)?;
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
