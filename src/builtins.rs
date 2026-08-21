use crate::error::{ShellError, ShellResult};
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

    pub fn execute(state: &mut ShellState, args: &[String]) -> ShellResult<i32> {
        if args.is_empty() {
            return Ok(0);
        }

        let cmd = args[0].as_str();
        let cmd_args = &args[1..];

        match cmd {
            "cd" => Ok(Self::builtin_cd(state, cmd_args)),
            "pwd" => Ok(Self::builtin_pwd()),
            "echo" => Self::builtin_echo(cmd_args),
            "exit" => Self::builtin_exit(state, cmd_args),
            "help" => Ok(Self::builtin_help()),
            "clear" => Self::builtin_clear(),
            "type" => Ok(Self::builtin_type(cmd_args)),
            "which" => Ok(Self::builtin_which(cmd_args)),
            "env" => Ok(Self::builtin_env()),
            "export" | "setenv" => Ok(Self::builtin_export(cmd_args)),
            "unset" | "unsetenv" => Ok(Self::builtin_unset(cmd_args)),
            "history" => Ok(Self::builtin_history(state)),
            "touch" => Ok(Self::builtin_touch(cmd_args)),
            "cat" => Self::builtin_cat(cmd_args),
            "true" => Ok(0),
            "false" => Ok(1),
            _ => Err(ShellError::BuiltinError(format!("unknown builtin: {cmd}"))),
        }
    }

    fn builtin_cd(state: &mut ShellState, args: &[String]) -> i32 {
        let target_path = if args.is_empty() || args[0] == "~" {
            env::var("HOME").unwrap_or_else(|_| "/".to_string())
        } else if args[0] == "-" {
            if let Some(prev) = &state.old_pwd {
                let path_str = prev.to_string_lossy().to_string();
                println!("{path_str}");
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

    fn builtin_pwd() -> i32 {
        match env::current_dir() {
            Ok(path) => {
                println!("{}", path.display());
                0
            }
            Err(e) => {
                eprintln!("sibsh: pwd: {e}");
                1
            }
        }
    }

    fn builtin_echo(args: &[String]) -> ShellResult<i32> {
        let mut no_newline = false;
        let mut start_idx = 0;

        if !args.is_empty() && args[0] == "-n" {
            no_newline = true;
            start_idx = 1;
        }

        let output = args[start_idx..].join(" ");
        if no_newline {
            print!("{output}");
            io::stdout().flush()?;
        } else {
            println!("{output}");
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

    fn builtin_help() -> i32 {
        println!("\x1b[1msibsh - Something Is Better Shell (Phase 1.1)\x1b[0m");
        println!("Type program names and arguments, then hit enter.\n");
        println!("Built-in Commands:");
        println!("  cd <dir>            Change working directory (supports ~, -)");
        println!("  pwd                 Print current working directory");
        println!("  echo [-n] <args>    Print text to stdout");
        println!("  exit [code]         Exit the shell");
        println!("  clear               Clear the terminal screen");
        println!("  type <cmd>          Describe how command would be interpreted");
        println!("  which <cmd>         Locate a program in $PATH");
        println!("  env                 Display all environment variables");
        println!("  export KEY=VAL      Set an environment variable");
        println!("  unset KEY           Remove an environment variable");
        println!("  history             Show command history");
        println!("  touch <file...>     Create or update file timestamp");
        println!("  cat <file...>       Concatenate and print file contents");
        println!("  true / false        Return exit status 0 or 1");
        println!("  help                Show this help message");
        0
    }

    fn builtin_clear() -> ShellResult<i32> {
        print!("\x1b[2J\x1b[H");
        io::stdout().flush()?;
        Ok(0)
    }

    fn builtin_type(args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("sibsh: type: missing operand");
            return 1;
        }

        let mut status = 0;
        for target in args {
            if Self::is_builtin(target) {
                println!("{target} is a shell builtin");
            } else if let Some(path) = Self::resolve_in_path(target) {
                println!("{target} is {}", path.display());
            } else {
                eprintln!("sibsh: type: {target}: not found");
                status = 1;
            }
        }
        status
    }

    fn builtin_which(args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("sibsh: which: missing operand");
            return 1;
        }

        let mut status = 0;
        for target in args {
            if let Some(path) = Self::resolve_in_path(target) {
                println!("{}", path.display());
            } else {
                status = 1;
            }
        }
        status
    }

    fn builtin_env() -> i32 {
        for (key, val) in env::vars() {
            println!("{key}={val}");
        }
        0
    }

    fn builtin_export(args: &[String]) -> i32 {
        if args.is_empty() {
            return Self::builtin_env();
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

    fn builtin_history(state: &ShellState) -> i32 {
        for (idx, cmd) in state.history.iter().enumerate() {
            println!("{:5}  {cmd}", idx + 1);
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

    fn builtin_cat(args: &[String]) -> ShellResult<i32> {
        if args.is_empty() {
            let mut buffer = String::new();
            io::stdin().read_to_string(&mut buffer)?;
            print!("{buffer}");
            return Ok(0);
        }

        let mut status = 0;
        for path_str in args {
            match fs::read_to_string(path_str) {
                Ok(contents) => print!("{contents}"),
                Err(e) => {
                    eprintln!("sibsh: cat: {path_str}: {e}");
                    status = 1;
                }
            }
        }
        io::stdout().flush()?;
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
