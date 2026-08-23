# sibsh

sibsh (Something Is Better Shell) is a lightweight, zero-dependency Unix shell written in Rust.

The goal is to build a fast, reliable shell from scratch using only the Rust standard library.

## Features (Phase 1.2 + additions)

- **Zero External Dependencies**: Built entirely using Rust standard library (`std`).
- **Interactive REPL**: Read-Eval-Print loop with custom prompt, error status display, and EOF handling.
- **Command Parser**: Supports single quotes (`'...'`), double quotes (`"..."`), escape characters, and environment variable expansion (`$VAR`, `$?`).
- **I/O Redirection**: `cmd > file` (create/truncate), `cmd >> file` (append), and `cmd < file` (stdin) — for external commands **and** built-ins, with quote-aware parsing so `echo 'a > b'` stays literal.
- **Tab Completion**: First Tab completes built-ins, aliases, `$PATH` executables, and file paths; a second Tab lists all candidates. Includes line editing (arrows, Home/End, backspace) and Up/Down history navigation.
- **Configuration File**: `~/.sibsh/sibsh.toml` sets the prompt template, history limit, startup aliases, and imports of bash/zsh rc files — parsed by a small TOML-subset parser, still zero dependencies.
- **Aliases**: Define with `alias name='value'`, remove with `unalias`; loaded from config at startup.
- **External Command Execution**: Resolves binaries in `$PATH` and handles process execution with standard I/O inheritance.
- **In-Memory History**: Tracks commands entered during the active session.
- **19 Built-in Commands**: Essential shell commands implemented natively.

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
| `cat [file...]` | Output file contents to standard output (or read from stdin); byte-safe |
| `alias [name='value']` | List all aliases, or define one at runtime |
| `unalias <name>` | Remove an alias |
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

# I/O redirection
echo "Hello, World" > greeting.txt
cat greeting.txt
echo "Second line" >> greeting.txt
sort < names.txt > sorted.txt
wc -l < sorted.txt

# Quoted operators are literal text
echo 'a > b'    # prints: a > b

# Aliases
alias ll='ls -la'
ll
unalias ll

# Exit session
exit 0
```

## Tab Completion

Press Tab to complete the current word:

- The first word completes built-ins, aliases, then executables found in `$PATH`.
- Other words complete filesystem paths; directories get a trailing `/` so the next Tab completes deeper.
- Hidden files only match when the typed prefix starts with `.`.
- A unique match completes with a trailing space; pressing Tab again lists all candidates in columns.

While typing you can also use: Backspace/Delete, Left/Right arrows, Home/End (Ctrl+A/E), Up/Down for history navigation, and Ctrl+C to clear the current line.

## Configuration

sibsh reads `$SIBSH_CONFIG` or, by default, `~/.sibsh/sibsh.toml` at startup:

```toml
prompt = "{user}@{host}:{cwd} ❯ "   # placeholders: {user}, {host}, {cwd}, {status}
history_limit = 1000               # cap on in-memory history
imports = ["~/.bashrc", "~/.zshrc"] # shell files run at startup

[aliases]
ll = "ls -la"
gs = "git status"
```

Imported files run bashrc/zshrc style: `export KEY=VAL`, `alias`, comments, and plain command lines execute; bash-only syntax is skipped silently so shared rc files work for the parts sibsh supports.

A missing or invalid config never prevents startup; parse errors report the line number.

On first run sibsh writes a fully commented template to `~/.sibsh/sibsh.toml` explaining every option. The same file ships in the repository as [`sibsh.toml.example`](sibsh.toml.example) — copy it to `~/.sibsh/sibsh.toml` and edit away.

## Roadmap

- [x] **Phase 1.1**: Core REPL loop, argument parser, built-in commands, process execution.
- [x] **Phase 1.2**: I/O Redirection (`>`, `>>`, `<`) for external commands and built-ins.
  - Quote-aware parsing (`echo 'a > b'` is literal), attached (`>file`) and spaced (`> file`) forms, `$VAR` expansion inside filenames, clear errors for missing filenames or unreadable/unwritable targets, and byte-safe `cat`.
- [x] **Additions (v0.1.3)**: Tab completion with line editing and history navigation; `~/.sibsh/sibsh.toml` configuration (prompt template, history limit); aliases (`alias`/`unalias`) with bashrc/zshrc-style imports.
- [ ] **Phase 1.3**: Pipelines (`cmd1 | cmd2 | cmd3`).
- [ ] **Phase 2.0**: Job control, signal handling (`SIGINT`, `SIGTSTP`), persistent history file.

## License

This project is licensed under the Apache License 2.0. See the [LICENSE](LICENSE) file for details.

