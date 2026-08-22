# Changelog

All notable changes to `sibsh` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned
- **Phase 1.3** — Pipelines (`cmd1 | cmd2 | cmd3`) with concurrent stage execution.
  See [CHECKLISTS.md → Phase 1.3](CHECKLISTS.md#13--pipelines--next-phase).
- **Phase 1.4** — Command sequencing and conditional execution (`;`, `&&`, `||`).
  See [CHECKLISTS.md → Phase 1.4](CHECKLISTS.md#14--command-sequencing--conditional-execution-planned-v02x).
- **Phase 1.5** — Filename expansion (globbing) and tilde expansion.
  See [CHECKLISTS.md → Phase 1.5](CHECKLISTS.md#15--filename-expansion-globbing--tilde-expansion-planned-v030).
- **Phase 2.0** — Job control, signal handling (`SIGINT`, `SIGTSTP`),
  persistent history file.

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
- New `ShellError::Redirection` variant with shell-grade messages
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
- `help` output updated with redirection syntax (banner now says Phase 1.2).

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

[Unreleased]: https://github.com/pbarot2009/sibsh/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/pbarot2009/sibsh/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/pbarot2009/sibsh/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/pbarot2009/sibsh/releases/tag/v0.1.0
