# sibsh — Prompt Design Specification

Reference specification for the two-line segment prompt. The implementation
lives in [`src/prompt.rs`](src/prompt.rs); rendering states are verified by
the integration suite and `tests/pty_harness.py`.

## Visual Architecture

```
╭─ [sibsh] <user@host> <path> on <vcs_branch> <vcs_status> via <runtime> [<duration>] [<exit_code>]
╰─❯ <user_input>
```

Segments render only when relevant (starship / oh-my-posh style). The
`[sibsh]` badge is always present; every other segment appears only when its
trigger condition holds.

## All Operational States & Test Cases

### 1. Normal Clean State

```
╭─ [sibsh] ~/code/project on  main via  rs
╰─❯ cargo check
```

### 2. Dirty Working Tree & Upstream Divergence

```
╭─ [sibsh] ~/code/project on  dev [⇡2 ⇣1 !3 ?1] via  rs
╰─❯ git status
```

- `⇡2`: 2 commits ahead of remote.
- `⇣1`: 1 commit behind remote.
- `!3`: 3 modified / staged files.
- `?1`: 1 untracked file.

### 3. Command Failure / Non-Zero Exit Code

```
╭─ [sibsh] ~/code/project on  main [127 ✘]
╰─❭
```

- Exit code badge renders only on failure (`$? != 0`).
- Prompt symbol shifts from green `❯` to red `❭`.

### 4. Execution Timer Active (> 2000 ms)

```
╭─ [sibsh] ~/code/project on  main via  rs [3.42s ]
╰─❯
```

### 5. Deep Path Truncation (Viewport Protection)

```
╭─ [sibsh] …/compiler/parser/ast on  main
╰─❯
```

- Paths deeper than 3 levels contract leading segments to `…/`.
- Home directory replaces `$HOME` with `~`.

### 6. SSH / Remote Session Detection

```
╭─ [sibsh] user@host ~/code/project on  main
╰─❯
```

- `user@host` renders only if `$SSH_TTY` or `$SSH_CONNECTION` is set.

### 7. Superuser / Root Privileges

```
╭─ [sibsh] # /etc/systemd/system
╰─#
```

- In root mode (`UID == 0`), the prompt symbol shifts to `#` in bold gold
  and runtime segments are removed.

## Glyph & Symbol Mapping Table

| Element | Nerd Font Glyph | Unicode Code Point | ASCII Fallback (`icons = "ascii"`) | State / Trigger |
| :--- | :--- | :--- | :--- | :--- |
| **Top Connector** | `╭─` | `\u256D\u2500` | `+-` | Always |
| **Bottom Connector** | `╰─` | `\u2570\u2500` | `+-` | Always |
| **Success Pointer** | `❯` | `\u276F` | `>` | Exit code == 0 |
| **Error Pointer** | `❭` | `\u276D` | `>` | Exit code != 0 |
| **Root Pointer** | `#` | `\u0023` | `#` | UID == 0 |
| **Git Branch** | `` | `\uF418` | `git:` | Active git repo |
| **Untracked Flag** | `?` | `\u003F` | `?` | Untracked files present |
| **Modified Flag** | `!` | `\u0021` | `!` | Uncommitted changes |
| **Ahead Sync** | `⇡` | `\u21E1` | `^` | Commits ahead of upstream |
| **Behind Sync** | `⇣` | `\u21E3` | `v` | Commits behind upstream |
| **Duration Clock** | `󰅐` | `\U000F0150` | `s` | Execution > 2.0 s |
| **Go Runtime** | `󰟓` | `\U000F07D3` | `go` | `.go` / `go.mod` found |
| **C/C++ Runtime** | `󰙲` | `\U000F0672` | `c` | `.c` / `CMakeLists.txt` |
| **Rust Runtime** | `󱘗` | `\U000F1617` | `rs` | `Cargo.toml` found |
| **Failure Marker** | `✘` | `\u2718` | `x` | Exit code != 0 |

## ANSI Color Palette Reference

| Token Name | ANSI Escape Sequence | RGB Hex Value | Role |
| :--- | :--- | :--- | :--- |
| `CLR_FRAME` | `\033[38;5;242m` | `#6C6C6C` | Structural connectors (`╭─`, `╰─`, delimiters) |
| `CLR_BRAND` | `\033[1;38;5;141m` | `#B18AE0` | `[sibsh]` badge |
| `CLR_DIR` | `\033[1;38;5;75m` | `#5FADF2` | Working directory path |
| `CLR_GIT` | `\033[38;5;211m` | `#F5879B` | Branch name & icon |
| `CLR_GIT_FLAG` | `\033[1;38;5;214m` | `#FFAD33` | Dirty state indicators (`[!?⇡⇣]`) |
| `CLR_LANG` | `\033[38;5;114m` | `#87D787` | Runtime segment (`rs`, `go`, `c`) |
| `CLR_TIME` | `\033[38;5;222m` | `#FCE094` | Execution timer |
| `CLR_ERR` | `\033[1;38;5;203m` | `#FF5F5F` | Failure code & `✘` icon |
| `CLR_OK_PTR` | `\033[1;38;5;120m` | `#87FF87` | Ready prompt symbol (`❯`) |
| `CLR_RESET` | `\033[0m` | N/A | Terminal attribute reset |

## Configuration

All states can be exercised through `~/.sibsh/sibsh.toml`:

```toml
icons = "ascii"      # switch every glyph to its ASCII fallback
git_status = false   # hide the git segment entirely (skips the subprocess)
prompt = "..."       # custom single-line template overrides the default;
                     # placeholders: {user} {host} {cwd} {status} {branch}
```

Test determinism: `SIBSH_FORCE_NON_ROOT=1` pins non-root rendering so suites
pass identically on root and non-root machines.
