# sibsh

sibsh (Something Is Better Shell) is a lightweight, zero-dependency Unix shell written in Rust.

The goal is to build a fast, reliable shell from scratch using only the Rust standard library.

## Features (Phase 1.1)

- **Zero External Dependencies**: Built entirely using Rust standard library (`std`).
- **Interactive REPL**: Read-Eval-Print loop with custom prompt, error status display, and EOF handling.
- **Command Parser**: Supports single quotes (`'...'`), double quotes (`"..."`), escape characters, and environment variable expansion (`$VAR`, `$?`).
- **External Command Execution**: Resolves binaries in `$PATH` and handles process execution with standard I/O inheritance.
- **In-Memory History**: Tracks commands entered during the active session.
- **16 Built-in Commands**: Essential shell commands implemented natively.

## Built-in Commands

| Command | Description |
| :--- | :--- |
| `cd [dir]` | Change working directory (supports `~`, `-`, relative, and absolute paths) |
| `pwd` | Print current working directory |
| `echo [-n] [args]` | Print text to standard output |
| `exit [code]` | Exit the shell session with an optional exit code |
| `clear` | Clear the terminal screen |
| `type [cmd]` | Check if a command is a built-in or locate its binary path |
| `which [cmd]` | Print the full path of an executable from `$PATH` |
| `env` | List all current environment variables |
| `export [KEY=VAL]` | Set or update an environment variable |
| `unset [KEY]` | Remove an environment variable |
| `history` | List command history for the current session |
| `touch [file...]` | Create empty files or update file timestamps |
| `cat [file...]` | Output file contents to standard output (or read from stdin) |
| `true` | Return exit code 0 |
| `false` | Return exit code 1 |
| `help` | Display built-in command reference |

## Getting Started

### Prerequisites

- Rust 1.80+ (or latest stable toolchain)
- Cargo

### Installation

Clone the repository and build from source:

```bash
git clone https://github.com/pbarot2009/sibsh.git
cd sibsh
cargo build --release
```

The compiled binary will be located at `./target/release/sibsh`.

### Running

Start the shell directly using Cargo:

```bash
cargo run
```

Or run the compiled binary:

```bash
./target/release/sibsh
```

## Example Usage

```bash
# Inspect environment variables and working directory
pwd
echo "Running as user: $USER"

# Directory navigation
cd /tmp
pwd
cd -

# Manage variables
export PROJECT="sibsh"
echo $PROJECT

# Run external programs
ls -la
uname -a

# Check command types
type cd
type git
which git

# Inspect exit codes
false
echo $?

# Exit session
exit 0
```

## Roadmap

- [x] **Phase 1.1**: Core REPL loop, argument parser, built-in commands, process execution.
- [ ] **Phase 1.2**: I/O Redirection (`>`, `>>`, `<`).
- [ ] **Phase 1.3**: Pipelines (`cmd1 | cmd2 | cmd3`).
- [ ] **Phase 2.0**: Job control, signal handling (`SIGINT`, `SIGTSTP`), persistent history file.

## License

This project is licensed under the Apache License 2.0. See the [LICENSE](LICENSE) file for details.

