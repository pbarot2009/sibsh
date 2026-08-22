# sibsh — Project Checklists

Project checklists for every phase of the `sibsh` roadmap.
Check items off (`[x]`) as they are completed and verified. Each phase should be
fully checked before starting the next one.

> **Legend**
> - `[ ]` not started · `[x]` done & verified · `(?)` needs a design decision first

---

## Phase 0 — Project Setup & Tooling (complete)

- [x] `Cargo.toml` at the repository root with:
  - [x] `name = "sibsh"`, `version = "0.1.2"`, `edition = "2024"`
  - [x] `license = "Apache-2.0"` (matches README)
  - [x] `description` and `repository` metadata
  - [x] **No dependencies** — project policy is std-only (verified: Cargo.lock contains only sibsh)
- [x] Verified `cargo build --release` works cleanly (Rust 1.98, zero warnings)
- [x] `.gitignore` present (`/target`)
- [x] `LICENSE` file present (Apache-2.0)
- [ ] Optional CI: build + clippy + test on push (`.github/` exists; workflow TBD)

---

## Phase 1 — Core Shell (single command per line)

### 1.1 — REPL, Parser, Built-ins, Execution (complete)

- [x] Interactive REPL loop (`src/shell.rs`) with prompt render → read → history → dispatch
- [x] Colored prompt (`src/prompt.rs`): `user@host:path ❯` plus red `[status]` on failure
- [x] Graceful EOF (Ctrl+D) exit and `exit [code]` propagation to process exit code
- [x] Tokenizer (`src/parser.rs`):
  - [x] Single quotes `'...'` (literal)
  - [x] Double quotes `"..."` (with `\`, `\$`, `\"` escapes)
  - [x] Backslash escapes outside quotes
  - [x] Variable expansion `$VAR` and last-status `$?`
  - [x] Error on unclosed quote
- [x] External command execution (`src/executor.rs`) via `$PATH` lookup with inherited stdio
- [x] In-memory session history (`history` builtin)
- [x] Built-in commands (`src/builtins.rs`) — all verified working:
  - [x] `cd` (`~`, `-`/OLDPWD, relative, absolute) · `pwd`
  - [x] `echo [-n]` · `exit [code]` · `clear` · `help`
  - [x] `type` · `which` · `env` · `export`/`setenv` · `unset`/`unsetenv`
  - [x] `history` · `touch` · `cat` · `true` · `false`
- [x] Centralized error type (`src/error.rs`) with clean user-facing messages
- [x] README documents features, commands, usage, and roadmap

Known polish items carried into later phases:

- [x] ~~`cat` uses `read_to_string`~~ → now byte-safe via `io::copy` (fixed in 1.2)
- [ ] `touch` opens files but does not update mtimes of existing files (deferred to Phase 2.0)
- [ ] `setenv`/`unsetenv` aliases exist in code but are undocumented in README/help
- [ ] `export KEY` (no `=`) silently sets empty value — decide intended semantics

---

### 1.2 — I/O Redirection (done in v0.1.2)

Goal: support `cmd > file`, `cmd >> file`, `cmd < file`, and combinations,
for **both external commands and built-ins**, with correct precedence and
clear error messages.

#### Design decisions (settled)

- [x] Token-level detection: operators recognized inside the tokenizer's
      quote-state machine only when unquoted; quoted `'>'` stays a literal argument
- [x] Data model: `Redirection { mode: RedirectMode, path: String }` +
      `ParsedCommand { args, redirects }` in `parser.rs`
- [x] Multiple redirections per line supported; all targets opened in order,
      last one per stream wins (bash behavior)
- [x] Out of scope for 1.2 (defer): `2>` / `&>` fd numbers, heredocs `<<`,
      here-strings `<<<`, `n>&m` duplication, `<>`

#### Parser changes (`src/parser.rs`) (done)

- [x] Recognize `>` (truncate/create), `>>` (append/create), `<` (read) **only in
      `ParseState::Normal`** — never inside single/double quotes
- [x] Correctly distinguish `>>` from `>` (peek next char before emitting)
- [x] Support attached form (`>file`) and spaced form (`> file`) — both work
- [x] Operator terminates the current word (e.g. `echo hi>out` works)
- [x] Parse errors implemented:
  - [x] Missing filename: `echo hi >` → "expected filename after redirection"
  - [x] Empty result filename after expansion: `> "$EMPTY"` → same error
  - [x] Expansion containing spaces becomes a single path token (documented)
- [x] Public API changed to `parse(line, last_status) -> ParsedCommand`; all call sites updated
- [x] Expansions (`$VAR`, `$?`) work inside redirected filenames

#### Executor changes (`src/executor.rs`) (done)

- [x] Apply `<` : open file read-only, wired via `Stdio::from(File)`
- [x] Apply `>` : create/truncate, wire stdout
- [x] Apply `>>` : `OpenOptions::new().create(true).append(true)`
- [x] Missing input file → `sibsh: nosuchfile: No such file or directory`,
      status `1`, command does not run
- [x] Unwritable/uncreatable target → error, status `1`, command does not run
- [x] Inherited stderr and interactive stdin preserved for non-redirected streams
- [x] Exit status still propagates correctly through redirection

#### Built-in integration (`src/builtins.rs`, `src/shell.rs`) (done)

- [x] Stdout of built-ins redirects (`echo hi > f.txt` writes the file)
- [x] Implementation: explicit `Box<dyn Write>` output handle built per call from
      the redirected file (no global mutation); all printing builtins converted
      (`echo`, `pwd`, `env`, `help`, `history`, `cat`, `type`, `which`, `cd -`, `clear`)
- [x] Stdin of built-ins redirects (`cat < f.txt`); reader created lazily so only
      `cat` touches stdin
- [x] `export`, `unset`, `exit`, `true`, `false` honor side effects regardless of redirect
- [x] Redirection errors report like bash and never kill the REPL

#### REPL / state (`src/shell.rs`) (done)

- [x] Dispatch line as `ParsedCommand { args, redirects }`; `last_status` semantics intact
- [x] Failed redirection sets `last_status = 1` and continues the loop
- [x] History records the raw line exactly as typed (unchanged behavior)
- [x] Prompt/status indicator `[N]` reflects redirected-command failures correctly
- [x] REPL reads stdin per-call (lock no longer held across dispatch), fixing a
      deadlock where builtin execution could block on the held lock

#### Test matrix — all 15 cases verified passing

| # | Input | Result |
|---|-------|--------|
| 1 | `echo hello > out.txt` | file contains `hello\n`, nothing on terminal |
| 2 | `echo a > f` then `echo b >> f` | file contains `a\nb\n` |
| 3 | `echo x > f` then `echo y > f` | truncated to `y\n` |
| 4 | `cat < in.txt` | prints contents |
| 5 | `sort < unsorted.txt > sorted.txt` | both directions at once |
| 6 | `echo 'a > b'` | literal text, no redirect |
| 7 | `echo ">"` | literal `>` printed |
| 8 | `echo hi >out.txt` (no space) | works |
| 9 | `cat < missing.txt` | error, status 1, `cat` never runs |
| 10 | `echo hi > /root/nope/x` | path error, status 1 |
| 11 | `echo hi >` (nothing after) | syntax error message |
| 12 | `echo $USER > who.txt` | expansion in filename and content |
| 13 | `false > log.txt` | log created empty, status 1 |
| 14 | `cat < f.txt > g.txt` | g gets f's copy |
| 15 | `wc -l < f` via external cmd | works with real binaries |

| # | Input | Expected |
|---|-------|----------|
| 1 | `echo hello > out.txt` | file contains `hello\n`, nothing on terminal |
| 2 | `echo a > f` then `echo b >> f` | file contains `a\nb\n` |
| 3 | `echo x > f` then `echo y > f` | file truncated to `y\n` |
| 4 | `cat < in.txt` | prints contents |
| 5 | `sort < unsorted.txt > sorted.txt` | both directions at once |
| 6 | `echo 'a > b'` | literal text printed, NO redirect |
| 7 | `echo ">"` | literal `>` printed |
| 8 | `echo hi >out.txt` (no space) | works |
| 9 | `cat < missing.txt` | error, status 1, `cat` never runs |
| 10 | `echo hi > /root/nope/x` | permission/path error, status 1 |
| 11 | `echo hi >` (nothing after) | syntax error message |
| 12 | `echo $USER > who.txt` | expansion inside filename value AND content |
| 13 | `false > log.txt` | log created/empty, status 1 |
| 14 | `cat < f.txt > g.txt` | g gets f's copy |

#### Testing and verification for 1.2 (done)

- [x] Manually ran every row of the matrix above through the release binary
- [x] Added `tests/integration.rs`: 18 tests driving the real binary via stdin,
      covering the full matrix plus Phase 1.1 regressions (std-only)
- [x] 10 parser unit tests in `src/parser.rs`
- [x] `cargo clippy --all-targets`: 0 warnings
- [x] `cargo test`: 28/28 passing (10 unit + 18 integration)
- [x] README updated: Features section, redirection examples, roadmap box `[x]`
- [x] CHANGELOGS.md released as `[0.1.2] - 2026-08-22`; Cargo.toml bumped to 0.1.2
- [x] `help` output updated with redirection syntax

---

### 1.3 — Pipelines (next phase, planned for v0.2.0)

Goal: `cmd1 | cmd2 | cmd3` — stdout of each stage feeds stdin of the next,
stages run concurrently, std-only.

#### Design decisions (settle before coding)

- [ ] (?) Data model: extend `ParsedCommand` to a list of stages
      (`Vec<Stage>`, each stage = args + its own redirects), splitting on
      unquoted `|` during tokenization
- [ ] (?) Concurrency: one thread per stage with `Stdio::piped()` chaining
      (std-only); document thread-count cap if any
- [ ] (?) Builtins inside pipelines (`history | grep cd`): recommended approach
      is running the builtin in a worker thread writing into the pipe; decide
      and document
- [ ] Out of scope for 1.3 (defer): `|&` (stderr pipes), `>|` (noclobber
      override), pipeline negation `!`

#### Parser changes (`src/parser.rs`)

- [ ] Recognize `|` only in `ParseState::Normal` (quoted `'a | b'` stays literal)
- [ ] Split line into stages; operator terminates current word (`echo hi|wc -c` works)
- [ ] Syntax errors for: leading `|`, trailing `|` with no command,
      double operators (`a || b` reserved for Phase 1.4 — explicit error for now)
- [ ] Each stage keeps its own redirections (`cat < in | sort > out` parses correctly)

#### Execution engine (`src/shell.rs` or new `src/pipeline.rs`)

- [ ] Single-stage lines behave exactly as in 1.2 (no regression)
- [ ] Multi-stage: spawn all stages concurrently, wire pipes between them
- [ ] Close parent-side pipe fds promptly so stages see EOF (classic hang risk)
- [ ] Wait for every stage; exit status = **last** stage's status (bash semantics)
- [ ] Unknown command mid-pipeline reports error but pipeline still drains
- [ ] Builtins at any stage position work per design decision above
- [ ] Redirections bind tighter than pipes (`cmd > f | cmd2`: first stage writes
      to file) — verify matches bash

#### Test matrix (all cases must pass)

| # | Input | Expected |
|---|-------|----------|
| 1 | `echo hello \| cat` | prints `hello` |
| 2 | `printf 'b\na\n' \| sort` | prints `a\nb` |
| 3 | `cat file \| grep x \| wc -l` | 3-stage chain counts correctly |
| 4 | `yes \| head -5` | terminates (head closes pipe, yes gets SIGPIPE/dies cleanly) |
| 5 | `nosuchcmd \| cat` | error printed, `cat` exits 0, overall status from last stage |
| 6 | `history \| grep cd` | builtin feeds pipeline |
| 7 | `cat < in \| sort > out` | redirect + pipe mix works |
| 8 | `false \| true` | overall status 0 (last stage wins) |
| 9 | `true \| false` | overall status 1 |
| 10 | `echo 'a \| b'` | literal text, NO pipeline |

#### Testing & verification for 1.3

- [ ] Parser unit tests: stage splitting, quoted `\|`, syntax errors
- [ ] Integration tests covering the full matrix above via stdin scripts
- [ ] Stress: long chains (`\|` × 8) complete without deadlock/hang
- [ ] `cargo clippy --all-targets` clean, no new warnings
- [ ] README features/examples updated, roadmap box `[x]`
- [ ] CHANGELOGS entry → release as next patch/minor per SemVer plan

---

### 1.4 — Command Sequencing & Conditional Execution (planned v0.2.x)

Goal: run multiple commands on one line with `;`, `&&`, `||`, using bash
precedence and exit-status semantics.

#### Design decisions (settle before coding)

- [ ] (?) Precedence model: bash gives `&&`/`||` equal precedence,
      left-associative (`a && b \|\| c` runs `c` only if `b` failed);
      implement a small list-parser over stages — document it
- [ ] (?) Interaction with pipelines: a full pipeline is the unit between
      operators (`cmd1 \| cmd2 && cmd3`) — build 1.4 on top of 1.3, not beside it
- [ ] Out of scope for 1.4 (defer): grouping `( )`, backgrounding `&`,
      arithmetic/`[[ ]]` conditionals

#### Parser changes (`src/parser.rs`)

- [ ] Recognize `;`, `&&`, `||` only when unquoted (quoted `';'` / `'&&'` stay literal)
- [ ] Distinguish `&&` vs `&` (reserved), `||` vs `|` — peek-based, consistent with `>>`
- [ ] Syntax errors: empty command between/before/after operators
- [ ] Public API evolves to an ordered list of (pipeline, connector) items

#### Execution semantics (`src/shell.rs`)

- [ ] `;` — always run the next command regardless of prior status
- [ ] `&&` — run next only if previous status == 0
- [ ] `||` — run next only if previous status != 0
- [ ] Overall line status = status of the **last executed** command
- [ ] Short-circuiting must skip evaluation entirely (no side effects of skipped commands)
- [ ] Works across builtins and external commands uniformly
- [ ] `exit` mid-list still exits immediately

#### Test matrix (all cases must pass)

| # | Input | Expected |
|---|-------|----------|
| 1 | `true && echo yes` | prints `yes` |
| 2 | `false && echo yes` | prints nothing, status 1 |
| 3 | `false \|\| echo fallback` | prints `fallback` |
| 4 | `true \|\| echo no` | prints nothing |
| 5 | `false ; echo done` | prints `done` (semicolon ignores status) |
| 6 | `false && echo a \|\| echo b` | prints `b` (left-assoc) |
| 7 | `echo 'x ; y'` | literal `x ; y`, single command |
| 8 | `cd /tmp && pwd` | prints `/tmp` |
| 9 | `history ; pwd` | both builtins run |
| 10 | `echo hi > f && wc -l < f` | redirect inside a conditional chain |

#### Testing & verification for 1.4

- [ ] Parser unit tests: connectors, precedence order, quoted literals, errors
- [ ] Integration tests covering the full matrix above
- [ ] Regression: 1.2 redirections and 1.3 pipelines still pass
- [ ] `cargo clippy --all-targets` clean
- [ ] README + CHANGELOGS updated

---

### 1.5 — Filename Expansion (Globbing) & Tilde Expansion (planned v0.3.0)

Goal: `*`, `?`, and `[...]` wildcard expansion plus full tilde expansion,
applied after quoting/expansion but before execution.

#### Design decisions (settle before coding)

- [ ] (?) Architecture: post-tokenization expansion pass over `ParsedCommand`
      (recommended) vs in-tokenizer matching; glob must NOT apply to quoted tokens
- [ ] (?) No-match behavior: bash leaves the pattern literally (`echo *.nomatch`
      prints `*.nomatch`) — adopt that, document it
- [ ] (?) Hidden files: `*` does not match dotfiles (bash default) — adopt
- [ ] (?) Sort order: expanded matches sorted byte-wise (bash default) — adopt
- [ ] Out of scope for 1.5 (defer): `**` recursive glob, brace expansion `{a,b}`,
      extended globs `+(...)`, `~user` lookup beyond `$HOME`

#### Expansion module (new `src/expansion.rs` recommended)

- [ ] Tilde: leading `~` (unquoted) expands to `$HOME`; `~/sub` joins paths;
      track which chars were quoted so `"~"` stays literal
- [ ] Glob matcher over path segments: `*` (any run), `?` (single char),
      `[abc]`/`[a-z]`/`[!a-z]` character classes
- [ ] Pattern applies per path segment (`src/*.rs` matches one level only)
- [ ] Results sorted; duplicates avoided; directories not auto-filtered
- [ ] Applied to arguments AND redirection filenames (`> *.log` uses first match
      like bash) — verify against bash and document

#### Integration (`src/shell.rs`, `src/parser.rs`)

- [ ] Expansion pass runs between parse and dispatch, preserving arg order
- [ ] Quoted tokens bypass glob and tilde entirely (`echo "*"` prints `*`)
- [ ] Escaped metacharacters (`\*`, `'*)`) stay literal
- [ ] Redirection targets expand too (`cat < data?.txt`)

#### Test matrix (all cases must pass)

| # | Input | Expected |
|---|-------|----------|
| 1 | `echo *.md` (matches exist) | sorted list of .md files |
| 2 | `echo *.nomatch` | prints literal `*.nomatch` |
| 3 | `echo '*'` | prints literal `*` |
| 4 | `echo ?at` where `cat.txt` exists | matches single-char patterns correctly |
| 5 | `touch a.txt b.txt && rm ?.txt` | removes exactly those two files |
| 6 | `ls src/*.rs` | lists Rust sources only |
| 7 | `echo ~` | prints `$HOME` |
| 8 | `echo ~/x` | prints `$HOME/x` |
| 9 | `echo "~"` | prints literal `~` |
| 10 | `echo .*` | includes only dotfiles actually present, no false matches |

#### Testing & verification for 1.5

- [ ] Unit tests for matcher (`*`, `?`, classes, negation) and tilde logic
- [ ] Integration tests covering the matrix using temp dirs created by the test itself
- [ ] Regression: phases 1.2–1.4 suites still green
- [ ] `cargo clippy --all-targets` clean
- [ ] README + CHANGELOGS updated

---

## Phase 2.0 — Interactivity & Robustness

- [ ] Signal handling: `SIGINT` (Ctrl+C kills child / clears line, not the shell),
      `SIGTSTP` (Ctrl+Z suspends child)
- [ ] Job control: `jobs`, `fg`, `bg`, background execution with `&`
- [ ] Persistent history file (`~/.sibsh_history`): load on start, append on exit,
      `HISTSIZE` cap, dedupe consecutive duplicates
- [ ] Line editing (arrow keys, home/end, history navigation) — evaluate whether
      to stay std-only (raw-termios via libc-free approach) or relax the
      zero-dependency policy; record the decision in README
- [ ] Scripting mode: `sibsh script.sh`, shebang support (`#!/usr/bin/env sibsh`)
- [ ] Logical operators `&&` and `||`, sequencing `;`
- [ ] Tilde completion of `~user` forms, globbing `*.txt` (design decision)
- [ ] Fix carried-over polish items from 1.1 (byte-safe `cat`, real `touch` mtimes)
- [ ] Full regression pass over Phases 1.1–1.3 matrices
- [ ] Release `1.0.0` with updated README, CHANGELOGS, LICENSE

---

## Definition of Done (every phase) — met for Phases 0, 1.1, 1.2

- [x] All checklist items checked *(Phases 0, 1.1, 1.2)*
- [x] `cargo build --release` warning-free
- [x] `cargo clippy` clean (0 warnings, pedantic lints enabled)
- [x] Integration tests passing (28/28)
- [x] README + CHANGELOGS.md updated
- [x] Smoke-tested through a real terminal session (piped scripts + interactive prompt render)
