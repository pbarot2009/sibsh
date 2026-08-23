# Changelog

All notable changes to `sibsh` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned
- **Phase 1.4** — Command sequencing and conditional execution (`;`, `&&`, `||`).
  See [CHECKLISTS.md → Phase 1.4](CHECKLISTS.md#14--command-sequencing--conditional-execution-planned-v02x).
- **Phase 1.5** — Filename expansion (globbing) and tilde expansion.
  See [CHECKLISTS.md → Phase 1.5](CHECKLISTS.md#15--filename-expansion-globbing--tilde-expansion-planned-v030).
- **Phase 2.0** — Job control, signal handling (`SIGINT`, `SIGTSTP`),
  persistent history file.

## [0.2.1] - 2026-08-23

Resize survival for the line editor, plus the fix for duplicated prompt
frames while typing.

### Fixed

- **Duplicated prompt frames while typing**: the editor cached the terminal
  width once per prompt and trusted a remembered row count. After a window
  resize (or when a long line scrolled the screen), that geometry went stale
  and repaints anchored at the wrong row, stacking duplicate prompts. The
  painter now re-queries the terminal size before every paint and re-measures
  its region whenever the width changed.
- **Screen resize mid-line now preserves all state**: on the next keystroke
  after a resize the editor redraws exactly once under the new geometry with
  the typed buffer, cursor position, history position, saved live line, and
  pending completion candidates fully intact.
- **Repaints clamp to the viewport**: when the render is taller than the
  screen (long input forces scrolling), the cursor can no longer be moved past
  the top row into scrollback; the visible screen is cleared and rewritten
  instead of leaving frame debris.
- `stty` stderr noise (`Inappropriate ioctl for device`) no longer leaks into
  output when stdin is not a terminal (piped scripts, tests).

### Changed

- New `src/tty.rs` module: terminal size via `ioctl(TIOCGWINSZ)` declared
  directly against the platform libc — still zero external crates. Replaces
  one `stty size` process spawn per query; the editor now pays a single
  syscall per keystroke instead of a fork/exec, making prompt rendering
  faster as well as resize-correct. Falls back to parsing `stty size`, then
  to 24×80.
- `tests/pty_harness.py` grew to 26 interactive scenarios: the mini terminal
  emulator now tracks soft (wrap) vs hard (newline) line breaks and re-flows
  its grid on window resizes like a real terminal, replaying output and
  resizes chronologically. New scenarios cover shrinking and growing the
  window mid-typing, rapid successive resizes with an empty buffer, and input
  longer than the screen height. Verified against the v0.1.46 binary: the old
  build fails the frame-stability checks, the new build passes every
  scenario.
- Test coverage grew to 168 tests: 103 unit + 65 integration.

## [0.2.0] - 2026-08-23

Phase 1.3 — Pipelines.

### Added
- **Pipelines** (`cmd1 | cmd2 | cmd3`), std-only:
  - Stages run concurrently: external commands are spawned as processes
    chained through OS pipes created with `std::io::pipe`; all stages launch
    before any is reaped.
  - Built-ins work at any stage position (`history | grep cd`,
    `sort < f | cat`): each builtin stage runs in a dedicated worker thread
    against a snapshot of the shell state, so pipeline members cannot mutate
    the interactive shell (bash subshell semantics).
  - The pipeline's exit status is the last stage's status, matching bash;
    the prompt badge and `$?` expansion reflect it.
  - Unknown commands mid-pipeline report an error (stage status 127) while
    the pipeline still drains: later stages run, upstream writers hit EPIPE,
    downstream readers see EOF.
  - Redirections bind tighter than pipes, like bash: `echo hi > f | cat`
    writes the file and gives `cat` immediate EOF; a stage with its own
    `< file` stdin drops the pipe read end so upstream writers hit EPIPE.
  - Alias expansion applies to the first word of every stage.
- New syntax errors with bash-like messages: leading `|` ("unexpected '|'"),
  trailing or doubled `|` ("expected command after '|'"), and an explicit
  rejection of `||` (reserved for Phase 1.4). A pipe cutting short a
  redirect filename (`echo > | cat`) is a redirection syntax error.
- `src/pipeline.rs`: dedicated pipeline execution module documenting the
  concurrency model; `src/executor.rs` gained `StdinSource`/`StdoutSink`
  stream abstractions over files and pipe ends plus a non-waiting
  `Executor::spawn`; `help` output documents pipeline syntax.
- Prompt design specification document ([PROMPT.md](PROMPT.md)): visual
  architecture, all operational states, glyph mapping table, and ANSI color
  palette reference.

### Changed
- `Parser::parse` now returns one `ParsedCommand` per pipeline stage
  (`Vec<ParsedCommand>`); single-command lines yield exactly one element and
  behave identically to v0.1.46.
- Built-in execution is routed through explicit I/O streams (reader/writer
  handles) instead of assuming stdout/stdin, enabling redirection, pipes,
  and worker threads through one code path.
- Test coverage grew to 164 tests: 99 unit (parser, config parser,
  completion, prompt) and 65 integration tests driving the real binary,
  including the full Phase 1.3 matrix, stress chains (9 stages / 8 pipes,
  256 KiB transfers), and PTY harness scenarios.

## [0.1.46] - 2026-08-23

### Fixed

- **Misaligned two-line prompt**: in raw mode the kernel does not translate
  `\n` into carriage-return + linefeed, so the bottom prompt line started at
  whatever column the top line ended on. The editor now emits explicit
  `\r\n` for every newline it writes (prompt, Enter, Ctrl+C, candidate
  lists).
- **Stacked / duplicated prompts while typing**: repaints moved up only the
  prompt's newline count, ignoring rows added by terminal wrapping. A new
  `Painter` tracks exactly how many screen rows the previous render
  occupied (measured with a terminal-width query via `stty size` and
  per-character display widths, including wide CJK characters) and clears
  precisely that region before rewriting. Long lines, backspace storms,
  history swaps between entries of different lengths, Home/End/arrow moves
  inside wrapped lines, and completion candidate lists can no longer leave
  stale fragments behind.
- Cursor repositioning after edits now moves by display cells instead of
  character counts, keeping multibyte input aligned.
- Completion candidate columns are padded by display width so multibyte
  names stay aligned; column layout uses the real terminal width.
- Ctrl+C first clears the edited line, then prints the `^C` marker on a
  fresh row.

### Added

- `tests/pty_harness.py`: a pseudo-terminal test harness with a mini
  terminal emulator. It drives the real binary through fourteen interactive
  scenarios (wrapping, editing storms, history navigation, multibyte input,
  Ctrl+C, tab completion) and asserts exact screen state.

## [0.1.45] - 2026-08-23

### Added

- Two-line, segment-based prompt in the starship / oh-my-posh style:
  `╭─ [sibsh] ~/code/project on  main [⇡2 ⇣1 !3 ?1] via  rs [3.42s ]`
  with a `╰─❯` input line below.
- Git segment: current branch plus dirty-state flags (`!N` modified,
  `?N` untracked, `⇡N` ahead, `⇣N` behind) from one
  `git status --porcelain -b` call per prompt. Hidden outside repositories;
  disable entirely with `git_status = false`.
- Execution timer: commands taking longer than 2 seconds show `[3.42s 󰅐]`.
- Exit-code badge on failure: `[127 ✘]`, and the pointer switches from green
  `❯` to red `❭`.
- SSH session detection: `user@host` renders only when `SSH_TTY` or
  `SSH_CONNECTION` is set.
- Root mode (UID 0): bold gold `#` marker and pointer, runtime segments
  removed.
- Deep-path truncation: paths deeper than 3 levels contract leading
  segments to `…/`; `$HOME` still displays as `~`.
- Language detection by project files: Cargo.toml → rs, go.mod / *.go →
  go, CMakeLists.txt / C/C++ sources → c.
- Fixed ANSI-256 palette (frame gray, brand violet, directory blue, git
  pink, flag orange, language green, timer sand, error red, pointer green).
- `icons = "ascii"` config key switches every glyph to its plain ASCII
  fallback (`+-` frame, `>` pointer, `git:` prefix, `^`/`v` sync flags).
- `{branch}` placeholder for custom prompt templates.

### Changed

- The line editor now repaints multi-line prompts correctly: redraws move
  up over every prompt line (and any completion candidate list), clear to
  end of screen, then rewrite — no stacked or duplicated frames.
- Test suite grew to 131 tests (84 unit + 47 integration) covering every
  prompt state; tests pin environment identity so they pass identically as
  root or non-root, local or CI.

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

[Unreleased]: https://github.com/pbarot2009/sibsh/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/pbarot2009/sibsh/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/pbarot2009/sibsh/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/pbarot2009/sibsh/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/pbarot2009/sibsh/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/pbarot2009/sibsh/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/pbarot2009/sibsh/releases/tag/v0.1.0
