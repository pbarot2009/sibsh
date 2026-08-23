# sibsh

sibsh (Something Is Better Shell) is a lightweight, zero-dependency Unix shell written in Rust.

The goal is to build a fast, reliable shell from scratch using only the Rust standard library.

## Features (Phase 1.3)

- **Zero External Dependencies**: Built entirely using Rust standard library (`std`).
- **Pipelines**: `cmd1 | cmd2 | cmd3` chains stdout into stdin across any number of stages. Stages run concurrently — external commands as processes wired with OS pipes, built-ins in worker threads. The exit status is the last stage's, unknown commands report an error without stalling the pipeline, and redirections bind tighter than pipes (`cmd > f | cmd2` writes the file).
- **Interactive REPL**: Read-Eval-Print loop with a two-line segment prompt, error status display, and EOF handling.
- **Segment Prompt**: `╭─ [sibsh] ~/code/project on  main [⇡2 ⇣1 !3 ?1] via  rs [3.42s ]` over a `╰─❯` input line — git branch and dirty flags, SSH-only `user@host`, language detection (rs/go/c), 2-second execution timer, `[127 ✘]` exit badge with red pointer, root-mode `#`, deep-path truncation to `…/`, fixed ANSI-256 palette, and an `icons = "ascii"` fallback mode.
- **Command Parser**: Supports single quotes (`'...'`), double quotes (`"..."`), escape characters, and environment variable expansion (`$VAR`, `$?`).
- **I/O Redirection**: `cmd > file` (create/truncate), `cmd >> file` (append), and `cmd < file` (stdin) — for external commands **and** built-ins, with quote-aware parsing so `echo 'a > b'` stays literal.
- **Tab Completion**: First Tab completes built-ins, aliases, `$PATH` executables, and file paths; a second Tab lists all candidates. Includes line editing (arrows, Home/End, backspace) and Up/Down history navigation.
- **Resize-Safe Line Editor**: The editor re-queries the terminal size (single `ioctl(TIOCGWINSZ)` syscall) before every repaint. Resizing the window mid-line redraws once under the new geometry with the typed text, cursor position, history state, and pending completions fully preserved — no duplicated prompt frames. Renders taller than the screen clamp to the viewport instead of corrupting scrollback.
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

# Pipelines
echo hello | cat
printf 'b\na\n' | sort          # prints: a, then b
cat access.log | grep ERROR | wc -l   # stages run concurrently
history | grep cd               # built-ins work at any stage
cat < notes.txt | sort > sorted.txt   # redirects mix with pipes
false | true                    # status 0 (last stage wins)
true | false                    # status 1 (last stage wins)

# Aliases
alias ll='ls -la'
ll
unalias ll

# Exit session
exit 0
```

## Tab Completion

Press Tab to complete the current word:

- The first word completes built-ins, aliases, then executables found in `$PATH`; this applies to the first word of every pipeline stage.
- Other words complete filesystem paths; directories get a trailing `/` so the next Tab completes deeper.
- Hidden files only match when the typed prefix starts with `.`.
- A unique match completes with a trailing space; pressing Tab again lists all candidates in columns.

While typing you can also use: Backspace/Delete, Left/Right arrows, Home/End (Ctrl+A/E), Up/Down for history navigation, and Ctrl+C to clear the current line.

## Configuration

sibsh reads `$SIBSH_CONFIG` or, by default, `~/.sibsh/sibsh.toml` at startup:

```toml
# prompt = "{user}@{host}:{cwd} ❯" # custom single-line template;
                                    # placeholders: {user} {host} {cwd} {status} {branch}
icons = "ascii"                     # plain glyphs instead of Nerd Font symbols
git_status = true                   # false hides the git segment (and skips git subprocess)
history_limit = 1000                # cap on in-memory history
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
- [x] **Prompt redesign (v0.1.45)**: two-line segment prompt with git status, language detection, execution timer, exit badge, SSH detection, root mode, path truncation, ANSI-256 palette, and ASCII icon mode — full design specification in [`PROMPT.md`](PROMPT.md).
- [x] **Phase 1.3 (v0.2.0)**: Pipelines (`cmd1 | cmd2 | cmd3`).
  - Concurrent stage execution over std-only OS pipes, built-ins at any stage position via worker threads, per-stage aliases and redirections, bash status semantics (last stage wins), and clean draining when a command is missing mid-chain. `|&`, `>|`, and pipeline negation are deferred.
- [ ] **Phase 1.4**: Command sequencing and conditional execution (`;`, `&&`, `||`).
- [ ] **Phase 2.0**: Job control, signal handling (`SIGINT`, `SIGTSTP`), persistent history file.

## License

This project is licensed under the Apache License 2.0. See the [LICENSE](LICENSE) file for details.

