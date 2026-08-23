//! Pipeline execution: `cmd1 | cmd2 | ... | cmdN`.
//!
//! Every stage runs concurrently. External commands become processes whose
//! stdin/stdout are chained through OS pipes (`std::io::pipe`, std-only).
//! Builtin stages run in dedicated worker threads against a snapshot of the
//! shell state, so mutations inside a pipeline (`cd`, `export`, `exit`) do
//! not affect the interactive shell — mirroring how bash treats pipeline
//! members as subshells.
//!
//! The pipeline's exit status is the **last** stage's status (bash
//! semantics). A stage whose command cannot be found reports an error and
//! counts as status 127, but the pipeline still drains: later stages run,
//! upstream writers hit EPIPE, downstream readers see EOF.

use crate::builtins::Builtins;
use crate::error::ShellResult;
use crate::executor::{Executor, RedirectHandles, StdinSource, StdoutSink};
use crate::parser::ParsedCommand;
use crate::shell::ShellState;
use std::io;
use std::process::Child;
use std::thread::{self, JoinHandle};

/// A stage launched but not yet reaped.
enum Pending {
    /// An external process to wait on.
    Child(usize, Child),
    /// A builtin worker thread to join.
    Thread(usize, JoinHandle<i32>),
    /// A stage that finished without launching anything (redirect-only or
    /// failed spawn); its status is already known.
    Done(usize, i32),
}

/// Runs a multi-stage pipeline; `stages` must contain at least two entries
/// (single-stage lines bypass this module entirely).
pub fn run(state: &mut ShellState, stages: &[ParsedCommand]) -> ShellResult<i32> {
    debug_assert!(stages.len() >= 2, "single-stage lines bypass pipelines");
    let count = stages.len();

    // Alias expansion applies to the first word of every stage.
    let mut expanded: Vec<ParsedCommand> = stages.to_vec();
    for stage in &mut expanded {
        state.expand_first_alias(stage);
    }

    // Open every stage's file redirections up front (all targets are created
    // like bash); any failure aborts before a single stage starts.
    let mut handles: Vec<RedirectHandles> = expanded
        .iter()
        .map(|stage| RedirectHandles::open(&stage.redirects))
        .collect::<ShellResult<Vec<_>>>()?;

    // Wire inter-stage pipes. A stage with its own stdout redirection keeps
    // the file while the pipe's write end drops here, giving the next stage
    // immediate EOF (bash: `echo hi > f | cat`). Symmetrically, a stage with
    // its own stdin redirection drops the read end, so the upstream writer
    // hits EPIPE once everyone else closes.
    let mut pipes = Vec::with_capacity(count - 1);
    for _ in 0..count - 1 {
        pipes.push(io::pipe()?);
    }
    for (i, (reader, writer)) in pipes.into_iter().enumerate() {
        if !matches!(handles[i].stdout, StdoutSink::File(_)) {
            handles[i].stdout = StdoutSink::Pipe(writer);
        }
        if !matches!(handles[i + 1].stdin, StdinSource::File(_)) {
            handles[i + 1].stdin = StdinSource::Pipe(reader);
        }
    }

    // Launch all stages before waiting on any of them.
    let mut pending: Vec<Pending> = Vec::with_capacity(count);
    for (i, stage) in expanded.iter().enumerate() {
        if stage.args.is_empty() {
            // Redirection-only stage: targets were opened above; nothing runs.
            pending.push(Pending::Done(i, 0));
            continue;
        }
        let name = &stage.args[0];
        if Builtins::is_builtin(name) {
            let handle = &mut handles[i];
            let out = handle.builtin_writer()?;
            let mut input = handle.builtin_reader()?;
            let args = stage.args.clone();
            let mut snapshot = state.clone();
            let worker = thread::Builder::new()
                .name(format!("sibsh:{name}"))
                .spawn(move || {
                    Builtins::dispatch(&mut snapshot, &args, &mut input, out).unwrap_or_else(
                        |err| {
                            eprintln!("{err}");
                            1
                        },
                    )
                })?;
            pending.push(Pending::Thread(i, worker));
        } else {
            match Executor::spawn(name, &stage.args[1..], &mut handles[i]) {
                Ok(child) => pending.push(Pending::Child(i, child)),
                Err(err) => {
                    eprintln!("{err}");
                    pending.push(Pending::Done(i, 127));
                }
            }
        }
    }

    // Reap everything; the overall status comes from the final stage.
    let mut statuses = vec![0i32; count];
    for item in pending {
        match item {
            Pending::Done(i, code) => statuses[i] = code,
            Pending::Child(i, mut child) => {
                statuses[i] = child
                    .wait()
                    .ok()
                    .and_then(|status| status.code())
                    .unwrap_or(1);
            }
            Pending::Thread(i, worker) => statuses[i] = worker.join().unwrap_or(1),
        }
    }

    Ok(statuses[count - 1])
}
