// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! One way to run a process (@PLN184 Track P): `platform::process`.
//!
//! **The invariant:** every process loft starts goes through [`Spawn`], and a stop or loft's
//! own end stops the child together with everything it started, on every platform, unless the
//! spawn says [`Tree::Detached`].
//!
//! ```ignore
//! let r = Spawn::new(Program::path(&rustc))      // or Program::search("cargo"): left to PATH
//!     .arg("--edition=2024")
//!     .arg_path(&src)                             // a PathText, in the host's spelling
//!     .cwd(&dir)
//!     .run(b"")?;                                 // to completion, both streams drained
//! let mut p = Spawn::new(Program::search("loft")).arg("server.loft").start()?;
//! if !p.wait(Duration::from_secs(5)) { p.stop_tree()?; }
//! ```
//!
//! A run to completion follows the rule the `process` library states for loft scripts: **a
//! stream is never left without a reader.**  The moment the child exists, one thread drains
//! each output pipe and another feeds its input, so no pipe fills while another is waited on.

use crate::file_access::PathText;
use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The program a [`Spawn`] runs.
pub struct Program(std::ffi::OsString);

impl Program {
    /// A program named by its path, handed over in the host's spelling.
    #[must_use]
    pub fn path(path: &PathText) -> Program {
        Program(path.os_spelling().into_os_string())
    }

    /// A bare tool name (`"cargo"`): not a path, so it is left to the search path.
    #[must_use]
    pub fn search(name: &str) -> Program {
        Program(name.into())
    }

    /// A program the caller already holds as the OS spelled it (`current_exe`, `LOFT_BIN`).
    #[must_use]
    pub fn os(path: impl AsRef<OsStr>) -> Program {
        Program(path.as_ref().to_os_string())
    }
}

/// What a stop, and loft's own end, do to the processes a child starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tree {
    /// The child and everything it starts belong to this run: a stop ends them all, and so
    /// does loft's end.
    #[default]
    Owned,
    /// The child is started to outlive loft (the engine host's hot-swap target): no stop
    /// aimed at loft's own tree reaches it.
    Detached,
}

/// A process to start: the program, its arguments, where and how.
pub struct Spawn {
    cmd: Command,
    tree: Tree,
}

/// A finished run: the child's status and both streams as they arrived.  `timed_out` says the
/// run's limit ended it ([`Spawn::run_for`]): the tree was stopped.
pub struct Ran {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

impl Spawn {
    #[must_use]
    pub fn new(program: Program) -> Spawn {
        Spawn {
            cmd: Command::new(program.0),
            tree: Tree::Owned,
        }
    }

    /// One argument, passed as it is: never split, never quoted, never read as a path.
    #[must_use]
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Spawn {
        self.cmd.arg(arg);
        self
    }

    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Spawn
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.cmd.args(args);
        self
    }

    /// A path argument, handed over in the host's spelling.
    #[must_use]
    pub fn arg_path(mut self, path: &PathText) -> Spawn {
        self.cmd.arg(path.os_spelling());
        self
    }

    /// The child's working directory, handed over in the host's spelling.
    #[must_use]
    pub fn cwd(mut self, dir: &PathText) -> Spawn {
        self.cmd.current_dir(dir.os_spelling());
        self
    }

    #[must_use]
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Spawn {
        self.cmd.env(key, value);
        self
    }

    #[must_use]
    pub fn env_remove(mut self, key: impl AsRef<OsStr>) -> Spawn {
        self.cmd.env_remove(key);
        self
    }

    /// The child's input for [`Spawn::start`]; a run to completion feeds its own.
    #[must_use]
    pub fn stdin(mut self, s: Stdio) -> Spawn {
        self.cmd.stdin(s);
        self
    }

    /// The child's output for [`Spawn::start`]; a run to completion collects its own.
    #[must_use]
    pub fn stdout(mut self, s: Stdio) -> Spawn {
        self.cmd.stdout(s);
        self
    }

    /// The child's error stream for [`Spawn::start`]; a run to completion collects its own.
    #[must_use]
    pub fn stderr(mut self, s: Stdio) -> Spawn {
        self.cmd.stderr(s);
        self
    }

    #[must_use]
    pub fn tree(mut self, tree: Tree) -> Spawn {
        self.tree = tree;
        self
    }

    /// Start the child and hand back its handle.
    ///
    /// # Errors
    /// The OS's error when the program cannot be started.
    pub fn start(mut self) -> io::Result<Running> {
        let child = self.cmd.spawn()?;
        Ok(Running {
            child,
            tree: self.tree,
        })
    }

    /// Run to completion with `input` on stdin, both streams collected.
    ///
    /// # Errors
    /// The OS's error when the program cannot be started or waited on.
    pub fn run(self, input: &[u8]) -> io::Result<Ran> {
        self.run_limited(input, None)
    }

    /// [`Spawn::run`], but a child still running after `limit` is stopped with its tree, and
    /// the answer says `timed_out`.
    ///
    /// # Errors
    /// The OS's error when the program cannot be started or waited on.
    pub fn run_for(self, input: &[u8], limit: Duration) -> io::Result<Ran> {
        self.run_limited(input, Some(limit))
    }

    fn run_limited(mut self, input: &[u8], limit: Option<Duration>) -> io::Result<Ran> {
        // No input is an empty stdin: the child reads EOF at once, with no pipe to feed.
        self.cmd
            .stdin(if input.is_empty() {
                Stdio::null()
            } else {
                Stdio::piped()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut running = self.start()?;
        // Every reader exists before a byte is written, so a child that answers while it is
        // still being fed never blocks on a full pipe.
        let out = running.child.stdout.take();
        let err = running.child.stderr.take().map(drain);
        let feed = running.child.stdin.take().map(|mut pipe| {
            let input = input.to_vec();
            // A child that exits without reading its input closes the pipe: that is its
            // choice, not a failure of the run, so the write's error is not reported.
            std::thread::spawn(move || {
                let _ = pipe.write_all(&input);
            })
        });
        let mut stdout = Vec::new();
        let mut timed_out = false;
        let status = if let Some(limit) = limit {
            // A limit needs this thread free to watch the clock: stdout drains beside stderr.
            let out = out.map(drain);
            if !running.wait(limit) {
                timed_out = true;
            }
            let status = if timed_out {
                running.stop_tree()
            } else {
                running.finish()
            };
            if let Some(h) = out {
                stdout = h.join().unwrap_or_default();
            }
            status
        } else {
            // Without a limit, stdout drains on this thread: the third thread a run would
            // otherwise start costs as much as the read.
            if let Some(mut pipe) = out {
                let _ = pipe.read_to_end(&mut stdout);
            }
            running.finish()
        };
        if let Some(f) = feed {
            let _ = f.join();
        }
        let stderr = err.and_then(|h| h.join().ok()).unwrap_or_default();
        Ok(Ran {
            status: status?,
            stdout,
            stderr,
            timed_out,
        })
    }
}

fn drain(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = pipe.read_to_end(&mut out);
        out
    })
}

/// A started child.
pub struct Running {
    child: Child,
    tree: Tree,
}

impl Running {
    /// The child's process id.
    #[must_use]
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// How the child was started.
    #[must_use]
    pub fn tree(&self) -> Tree {
        self.tree
    }

    /// Is the child still running?
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Wait up to `limit` for the child to end: `true` when it did.
    pub fn wait(&mut self, limit: Duration) -> bool {
        let until = Instant::now() + limit;
        let mut step = Duration::from_millis(1);
        loop {
            if !self.alive() {
                return true;
            }
            let now = Instant::now();
            if now >= until {
                return false;
            }
            std::thread::sleep(step.min(until - now));
            step = (step * 2).min(Duration::from_millis(20));
        }
    }

    /// Wait for the child to end, however long that takes.
    ///
    /// # Errors
    /// The OS's error when the child cannot be waited on.
    pub fn finish(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }

    /// Stop the child.
    ///
    /// # Errors
    /// The OS's error when the child cannot be waited on.
    pub fn stop_tree(&mut self) -> io::Result<ExitStatus> {
        let _ = self.child.kill();
        self.child.wait()
    }

    /// The child's input pipe, when it was started with `Stdio::piped()`.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    /// The child's output pipe, when it was started with `Stdio::piped()`.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    /// The child's error pipe, when it was started with `Stdio::piped()`.
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }
}
