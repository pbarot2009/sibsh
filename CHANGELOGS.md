# Changelog

All notable changes to `sibsh` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
