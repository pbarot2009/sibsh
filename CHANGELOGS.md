# Changelog

All notable changes to `sibsh` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned
- **Phase 1.3** — Pipelines (`cmd1 | cmd2 | cmd3`) with concurrent stage execution.
  See [CHECKLISTS.md → Phase 1.3](CHECKLISTS.md#13--pipelines-next-phase-planned-for-v020).
- **Phase 1.4** — Command sequencing and conditional execution (`;`, `&&`, `||`).
  See [CHECKLISTS.md → Phase 1.4](CHECKLISTS.md#14--command-sequencing--conditional-execution-planned-v02x).
- **Phase 1.5** — Filename expansion (globbing) and tilde expansion.
  See [CHECKLISTS.md → Phase 1.5](CHECKLISTS.md#15--filename-expansion-globbing--tilde-expansion-planned-v030).
- **Phase 2.0** — Job control, signal handling (`SIGINT`, `SIGTSTP`),
  persistent history file.

## [0.1.4] - 2026-08-23

### Fixed

- Interactive mode showed two prompts on one line: both the REPL and the line
  reader painted it. Prompt rendering now belongs entirely to the line reader
  (`completion::read_line`), matching zsh's ZLE / fish model where the editor is
  the sole owner of prompt painting. Every redraw starts at column 0 with a
  single prompt.
- Non-terminal input (piped scripts) keeps its prompt output; the fallback path
  prints the prompt once before reading.
- CI failure `completes_builtin_commands`: completion unit tests depended on the
  real `$PATH` of the machine (GitHub runner images ship different tools). New
  `complete_in()` takes an explicit directory list; tests use an empty or
  synthetic one so results are identical everywhere.
- `cargo fmt --all` applied to all sources.

## [0.1.3] - 2026-08-23

Tab completion, runtime configuration, and aliases.

### Added
- **Tab completion** (`src/completion.rs`), std-only via `stty` raw mode:
  - First word completes builtins, aliases, then `$PATH` executables.
  - Other words complete filesystem paths; directories get a trailing `/`
    so the next Tab completes deeper. Hidden files only match when the
    typed prefix starts with `.`.
  - A single unambiguous command completes with a trailing space; a second
    Tab press lists all candidates in columns.
  - Line editing while reading: backspace/delete, left/right arrows,
    Home/End (Ctrl+A/E), and Up/Down history navigation with a saved
    in-progress line.
  - Ctrl+C clears the current line without exiting; Ctrl+D on an empty line
    exits as before. Non-terminal stdin (scripts, tests) automatically uses
    plain buffered reading.
- **Configuration file** `~/.sibsh/sibsh.toml` (`src/config.rs`), parsed by a
  small TOML-subset parser — still zero external dependencies:
  - `prompt = "... "` template with `{user}`, `{host}`, `{cwd}`, `{status}`
    placeholders.
  - `history_limit = N` caps in-memory history.
  - `[aliases]` table defines startup aliases.
  - `imports = ["~/.bashrc", "~/.zshrc"]` executes shell files at startup,
    bashrc/zshrc style: `export`, `alias`, comments, and plain command lines
    run; bash-only syntax is skipped silently so shared rc files work for the
    parts sibsh supports.
  - Path can be overridden with `$SIBSH_CONFIG` (used by tests). A missing or
    invalid file never prevents startup; parse errors report the line number.
  - On first run a fully commented template is written to the config path so
    every option and its syntax is documented in place. The same template
    ships as `sibsh.toml.example` in the repository.
- **Aliases**: new `alias` builtin (list all, or define with
  `alias name='value'`) and `unalias name`. The first word of a command
  expands once against these definitions (no recursion).

### Changed
- `touch` now updates the modification time of existing files (via
  `File::set_modified`) instead of only creating them.
- Prompt rendering supports config templates (`Prompt::render_with`);
  the default prompt is unchanged.
- Test coverage grew to 100 tests: 60 unit (parser, config parser, completion)
  and 40 integration tests driving the real binary.

## [0.1.2] - 2026-08-22

Phase 1.2 — I/O Redirection.

### Added
- **I/O Redirection** for external commands *and* built-ins:
  - `cmd > file` — create/truncate stdout target.
  - `cmd >> file` — create/append stdout target.
  - `cmd < file` — wire stdin from an existing file.
  - Examples: `echo hi > f.txt`, `cat < in.txt > out.txt`, `sort < unsorted > sorted`.
- Multiple redirections per line; all targets are opened in order (like bash),
  last one per stream wins.
- Quote-aware operator parsing: `>`, `>>`, `<` are only recognized when
  unquoted, so `echo 'a > b'` and `echo ">"` remain literal text. Attached
  (`>file`) and spaced (`> file`) forms both work, and an operator terminates
  the current word (`echo hi>out`).
- `$VAR` / `$?` expansion inside redirection filenames; quoted filenames may
  contain spaces (`> "my file.txt"`).
- New syntax errors: missing filename after an operator (`echo hi >`) and an
  expansion producing an empty filename (`> "$EMPTY"`).
- New `ShellError::Redirection` variant with clear messages
  (`sibsh: path: No such file or directory`); failed redirections skip the
  command entirely and set exit status `1`.
- Test coverage: 10 parser unit tests plus an 18-case integration suite
  (`tests/integration.rs`) driving the real binary through the full Phase 1.2
  edge-case matrix from CHECKLISTS.md.

### Changed
- All printing built-ins (`echo`, `pwd`, `env`, `help`, `history`, `cat`,
  `type`, `which`, `cd -`, `clear`) write through an explicit output handle
  instead of global stdout, enabling clean redirection without global mutation.
- `cat` is now byte-safe (`io::copy`) so binary files survive intact.
- REPL reads stdin per-call instead of holding the lock across command
  dispatch, letting built-ins read stdin safely.
- `help` output updated with redirection syntax.

### Fixed
- Deadlock where any built-in invocation could block on the stdin lock already
  held by the REPL loop.

## [0.1.1] - 2026-08-21

### Added
- Project documentation: `CHECKLISTS.md` with complete phase-by-phase roadmaps,
  design decisions, edge-case matrices, and definitions of done for every
  planned milestone (Phases 0, 1.1–1.5, 2.0).

## [0.1.0] - 2026-08-21

### Added
- Core interactive REPL loop with colored prompt (user, hostname, path, and error code).
- Tokenizer supporting single quotes, double quotes, escape sequences, and `$VAR` / `$?` expansions.
- PATH lookup and external process execution using standard library primitives.
- In-memory command history tracking (`history`).
- 16 initial built-in commands:
  - `cd`, `pwd`, `echo`, `exit`, `help`, `clear`
  - `type`, `which`, `env`, `export`, `unset`
  - `history`, `touch`, `cat`, `true`, `false`
- Graceful exit on EOF (Ctrl+D) and custom exit status propagation.

[Unreleased]: https://github.com/pbarot2009/sibsh/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/pbarot2009/sibsh/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/pbarot2009/sibsh/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/pbarot2009/sibsh/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/pbarot2009/sibsh/releases/tag/v0.1.0
