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
//! **`Tree::Owned`, per platform:**
//! - **Linux:** the child leads a process group of its own, so a stop reaches everything it
//!   started ([`Running::stop_tree`] signals the group), and `PR_SET_PDEATHSIG` sends it
//!   `SIGTERM` when loft ends, however loft ends.  A child that is loft passes that on to its
//!   own owned groups, so the guarantee reaches down a tree of loft processes.
//! - **Windows:** a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (see the Windows
//!   half below): a stop terminates the job, and loft's end closes it.
//! - **macOS and the other unixes:** the process group, and the stop through it.  Driver
//!   death is @PLN184's open question 5: macOS has no `PR_SET_PDEATHSIG`, so an owned tree is
//!   stopped by loft's own ends that run code (a normal exit, `SIGINT`/`SIGTERM`, which are
//!   forwarded) but not when loft itself is `SIGKILL`ed.  It stays so until the owner decides
//!   between a watcher process per tree and an exemption.
//!
//! **The terminal.**  A child in a group of its own has left the terminal's foreground group,
//! so a `Ctrl-C` no longer reaches it directly.  loft therefore forwards the terminal's
//! signals (`SIGINT`, `SIGQUIT`, `SIGHUP`) and `SIGTERM` to every owned group it holds, then
//! lets the signal do to loft what it would have done.  An owned child must not read the
//! terminal: a background group that does is stopped by `SIGTTIN`.  A run to completion never
//! hands it the terminal (its input is a pipe or nothing).  A child that IS handed the
//! terminal — the program a `loft` run is, a script that may prompt, a `loft` that runs a
//! user's tests — is started [`Tree::Foreground`]: owned in every other respect, but kept in
//! loft's own group, so it reads the terminal as loft would and a `Ctrl-C` reaches it there.
//!
//! **Only here.**  `std::process::Command::new` is refused by Clippy everywhere else
//! (`clippy.toml`); a test harness that must hold the raw process it inspects takes
//! [`harness_command`], which nothing in `src/` may call.  The guard is
//! `src/platform_process_guard.rs`.
//!
//! A run to completion follows the rule the `process` library states for loft scripts: **a
//! stream is never left without a reader.**  The moment the child exists, one thread drains
//! each output pipe and another feeds its input, so no pipe fills while another is waited on.

use crate::file_access::{Flavor, PathText};
use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The spelling a child is handed for `path`, so that it reaches the file the path names.
///
/// A host path is handed over as the OS spells it.  Under the emulated Windows host
/// (`LOFT_POISON_HOST=windows`) a PROGRAM's path is in Windows' spelling (`L:\a\b`, or `/a/b`
/// on the current drive), and the child — a real process on the real file system — is handed
/// the real path it names (`/a/b`), as a file operation is.  Windows' name rules were applied
/// when the program's path was made (`PathText::program`); the case-blind match of an
/// existing name that `file_access` applies to its own operations is not applied here.
///
/// # Errors
/// A path of a platform this is not (a test's Windows path on Linux), or another drive than
/// the emulated host's.
pub fn host_spelling(path: &PathText) -> io::Result<OsString> {
    if path.flavor() == Flavor::HOST {
        Ok(path.os_spelling().into_os_string())
    } else if path.flavor() == Flavor::program_host() {
        path.from_emulated()
            .map(|real| real.os_spelling().into_os_string())
            .map_err(|why| io::Error::new(io::ErrorKind::NotFound, why))
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{}: not a path on this platform", path.portable()),
        ))
    }
}

/// The program a [`Spawn`] runs.
pub struct Program(io::Result<OsString>);

impl Program {
    /// A program named by its path, handed over in the host's spelling ([`host_spelling`]).
    #[must_use]
    pub fn path(path: &PathText) -> Program {
        Program(host_spelling(path))
    }

    /// A bare tool name (`"cargo"`): not a path, so it is left to the search path.
    #[must_use]
    pub fn search(name: &str) -> Program {
        Program(Ok(name.into()))
    }

    /// A program the caller already holds as the OS spelled it (`current_exe`, `LOFT_BIN`).
    #[must_use]
    pub fn os(path: impl AsRef<OsStr>) -> Program {
        Program(Ok(path.as_ref().to_os_string()))
    }
}

/// What a stop, and loft's own end, do to the processes a child starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tree {
    /// The child and everything it starts belong to this run: a stop ends them all, and so
    /// does loft's end.
    #[default]
    Owned,
    /// [`Tree::Owned`] for a child handed loft's terminal: it stays in loft's own process
    /// group (the terminal's foreground group when loft is), so it can read the terminal and
    /// a `Ctrl-C` reaches it directly, where an owned group of its own would be stopped by
    /// `SIGTTIN`.  Driver death is as owned (Linux `PR_SET_PDEATHSIG`, the Windows job).  A
    /// stop ends the child, and on Windows its job; on unix it does not reach what the child
    /// started, because the group it would signal is loft's own.
    Foreground,
    /// The child is started to outlive loft (the engine host's hot-swap target): no stop
    /// aimed at loft's own tree, and not loft's end, reaches it.  Its own stop still reaches
    /// what it started.
    Detached,
}

/// A process to start: the program, its arguments, where and how.  A path that cannot be
/// handed over is kept as the error [`Spawn::start`] answers, so a chain needs no `?` per
/// argument.
pub struct Spawn {
    cmd: Command,
    tree: Tree,
    error: Option<io::Error>,
    /// The streams the caller chose (stdin, stdout, stderr): a run to completion leaves
    /// them as chosen, as `std`'s `output()` does.
    chosen: [bool; 3],
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
    #[allow(
        clippy::disallowed_methods,
        reason = "platform::process is the one home of Command::new (@PLN184 P6)"
    )]
    pub fn new(program: Program) -> Spawn {
        let (cmd, error) = match program.0 {
            Ok(p) => (Command::new(p), None),
            Err(e) => (Command::new(""), Some(e)),
        };
        Spawn {
            cmd,
            tree: Tree::Owned,
            error,
            chosen: [false; 3],
        }
    }

    fn path_or_error(&mut self, path: &PathText) -> Option<OsString> {
        match host_spelling(path) {
            Ok(p) => Some(p),
            Err(e) => {
                self.error.get_or_insert(e);
                None
            }
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

    /// [`Spawn::arg`] in place, for a command assembled across branches and helpers.
    pub fn push_arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Spawn {
        self.cmd.arg(arg);
        self
    }

    /// [`Spawn::args`] in place.
    pub fn push_args<I, S>(&mut self, args: I) -> &mut Spawn
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.cmd.args(args);
        self
    }

    /// [`Spawn::env`] in place.
    pub fn push_env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Spawn {
        self.cmd.env(key, value);
        self
    }

    /// The program as the child is handed it, for a diagnostic that shows the invocation.
    #[must_use]
    pub fn get_program(&self) -> &OsStr {
        self.cmd.get_program()
    }

    /// The arguments so far, for a diagnostic that shows the invocation.
    #[must_use]
    pub fn get_args(&self) -> std::process::CommandArgs<'_> {
        self.cmd.get_args()
    }

    /// A path argument, handed over in the host's spelling ([`host_spelling`]).
    #[must_use]
    pub fn arg_path(mut self, path: &PathText) -> Spawn {
        if let Some(p) = self.path_or_error(path) {
            self.cmd.arg(p);
        }
        self
    }

    /// The child's working directory, handed over in the host's spelling.
    #[must_use]
    pub fn cwd(mut self, dir: &PathText) -> Spawn {
        if let Some(p) = self.path_or_error(dir) {
            self.cmd.current_dir(p);
        }
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

    /// The child's input.  Unset, [`Spawn::start`] hands it loft's own and a run to
    /// completion an empty one; a run given input feeds it whatever is set here.
    #[must_use]
    pub fn stdin(mut self, s: Stdio) -> Spawn {
        self.cmd.stdin(s);
        self.chosen[0] = true;
        self
    }

    /// The child's output.  Unset, [`Spawn::start`] hands it loft's own and a run to
    /// completion collects it; set, a run leaves it as set and answers it empty.
    #[must_use]
    pub fn stdout(mut self, s: Stdio) -> Spawn {
        self.cmd.stdout(s);
        self.chosen[1] = true;
        self
    }

    /// The child's error stream, as [`Spawn::stdout`].
    #[must_use]
    pub fn stderr(mut self, s: Stdio) -> Spawn {
        self.cmd.stderr(s);
        self.chosen[2] = true;
        self
    }

    #[must_use]
    pub fn tree(mut self, tree: Tree) -> Spawn {
        self.tree = tree;
        self
    }

    /// Start the child and hand back its handle.  The spawn stays as it was, so it can be
    /// started again (a retry after a rebuild).
    ///
    /// # Errors
    /// The OS's error when the program cannot be started, or why a path could not be handed
    /// over.
    pub fn start(&mut self) -> io::Result<Running> {
        if let Some(e) = &self.error {
            return Err(io::Error::new(e.kind(), e.to_string()));
        }
        let child = os::spawn(&mut self.cmd, self.tree)?;
        Ok(Running::new(child, self.tree))
    }

    /// Run to completion on the streams as set — loft's own where unset — and answer the
    /// status, as `std`'s `status()` does.
    ///
    /// # Errors
    /// The OS's error when the program cannot be started or waited on.
    pub fn status(&mut self) -> io::Result<ExitStatus> {
        self.start()?.finish()
    }

    /// Run to completion with `input` on stdin, collecting each output stream the spawn did
    /// not set ([`Spawn::stdout`]), as `std`'s `output()` does.
    ///
    /// # Errors
    /// The OS's error when the program cannot be started or waited on.
    pub fn run(&mut self, input: &[u8]) -> io::Result<Ran> {
        self.run_limited(input, None)
    }

    /// [`Spawn::run`], but a child still running after `limit` is stopped with its tree, and
    /// the answer says `timed_out`.
    ///
    /// # Errors
    /// The OS's error when the program cannot be started or waited on.
    pub fn run_for(&mut self, input: &[u8], limit: Duration) -> io::Result<Ran> {
        self.run_limited(input, Some(limit))
    }

    fn run_limited(&mut self, input: &[u8], limit: Option<Duration>) -> io::Result<Ran> {
        // Input is a pipe to feed.  No input is an empty stdin unless one was set: the child
        // reads EOF at once.
        if !input.is_empty() {
            self.cmd.stdin(Stdio::piped());
        } else if !self.chosen[0] {
            self.cmd.stdin(Stdio::null());
        }
        if !self.chosen[1] {
            self.cmd.stdout(Stdio::piped());
        }
        if !self.chosen[2] {
            self.cmd.stderr(Stdio::piped());
        }
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
            timed_out = !running.wait(limit);
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

/// A started child.  It is asked whether it ended without being reaped, so until it is
/// reaped its id is its own and a stop of its tree reaches nothing else.
pub struct Running {
    child: Child,
    tree: Tree,
    os: os::Tree,
    /// The status once reaped.
    status: Option<ExitStatus>,
}

impl Running {
    fn new(child: Child, tree: Tree) -> Running {
        let os = os::Tree::adopt(&child, tree);
        Running {
            child,
            tree,
            os,
            status: None,
        }
    }

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

    /// Is the child still running?  Asked without reaping it.
    pub fn alive(&mut self) -> bool {
        self.status.is_none() && !os::exited(&mut self.child, false)
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

    /// Wait for the child to end, however long that takes, and reap it.
    ///
    /// # Errors
    /// The OS's error when the child cannot be waited on.
    pub fn finish(&mut self) -> io::Result<ExitStatus> {
        if let Some(s) = self.status {
            return Ok(s);
        }
        os::exited(&mut self.child, true);
        self.reap()
    }

    /// Stop the child and everything it started, and reap it.  On unix the group is asked to
    /// end (`SIGTERM`) and killed (`SIGKILL`) once the child has ended or two seconds have
    /// passed, so the status usually reads as ended by `SIGTERM`.  A child already reaped
    /// answers its own status: once reaped, what it left behind can no longer be told apart
    /// from a stranger.
    ///
    /// # Errors
    /// The OS's error when the child cannot be waited on.
    pub fn stop_tree(&mut self) -> io::Result<ExitStatus> {
        if let Some(s) = self.status {
            return Ok(s);
        }
        self.os.stop(&mut self.child);
        let _ = self.child.kill();
        self.reap()
    }

    fn reap(&mut self) -> io::Result<ExitStatus> {
        // The tree lets go of the child's id first, while the unreaped child still holds it.
        self.os.release(&self.child);
        let s = self.child.wait()?;
        self.status = Some(s);
        Ok(s)
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

impl Drop for Running {
    /// A child that already ended is reaped.  One still running keeps running, as a dropped
    /// `std::process::Child` does, and its tree stays owned: loft's end still reaches it.
    fn drop(&mut self) {
        if self.status.is_none() && !self.alive() {
            let _ = self.reap();
        }
        self.os.keep();
    }
}

/// A raw `std::process::Command` for a TEST HARNESS (`tests/`): the process a test drives
/// loft through and inspects — its exit code, its streams, its process group — rather than
/// one loft starts.  Nothing in `src/` calls it (`src/platform_process_guard.rs` counts), so
/// every process loft itself starts still goes through [`Spawn`].
#[doc(hidden)]
#[must_use]
#[allow(
    clippy::disallowed_methods,
    reason = "platform::process is the one home of Command::new (@PLN184 P6)"
)]
pub fn harness_command(program: impl AsRef<OsStr>) -> Command {
    Command::new(program)
}

/// The code a loft script reads for `status`: the program's own, or `128 + n` for signal `n`.
pub fn exit_code(status: ExitStatus) -> i64 {
    status
        .code()
        .map_or_else(|| signal_code(status), |c| own_code(i64::from(c)))
}

/// The program's own exit code, in the one form every platform can give it.  A Windows child
/// built on MSYS or Cygwin (`sh`, the Git for Windows tools) that a signal ended exits with
/// `n << 8`, a value no Unix exit code can take (those are 0..=255), so it reads as the
/// `128 + n` the same program answers on Unix: `sh -c 'kill -TERM $$'` is 143 on both, not
/// 3840 on one (formal/paths.md: one program answers the same on every platform).
fn own_code(c: i64) -> i64 {
    own_code_on(c, cfg!(windows))
}

fn own_code_on(c: i64, windows: bool) -> i64 {
    if windows && c.trailing_zeros() >= 8 && (1..=64).contains(&(c >> 8)) {
        128 + (c >> 8)
    } else {
        c
    }
}

#[cfg(unix)]
fn signal_code(s: ExitStatus) -> i64 {
    use std::os::unix::process::ExitStatusExt;
    s.signal().map_or(-1, |n| 128 + i64::from(n))
}

#[cfg(not(unix))]
fn signal_code(_: ExitStatus) -> i64 {
    -1
}

#[cfg(unix)]
/// Arm this process to die when the process that started it dies.  Linux has it in one call
/// (`PR_SET_PDEATHSIG`); macOS watches the parent's exit with `kqueue` (`EVFILT_PROC` /
/// `NOTE_EXIT`) on a thread; elsewhere a thread checks for re-parenting.
pub fn die_with_parent() {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
    }
    #[cfg(target_os = "macos")]
    {
        let parent = unsafe { libc::getppid() };
        std::thread::spawn(move || unsafe {
            let kq = libc::kqueue();
            if kq < 0 {
                return;
            }
            let mut ev: libc::kevent = std::mem::zeroed();
            ev.ident = parent as libc::uintptr_t;
            ev.filter = libc::EVFILT_PROC;
            ev.flags = libc::EV_ADD | libc::EV_ONESHOT;
            ev.fflags = libc::NOTE_EXIT;
            let mut out: libc::kevent = std::mem::zeroed();
            // Registers the watch and blocks until the parent exits (or was already gone,
            // which the registration reports as an error — the same answer).
            libc::kevent(kq, &raw const ev, 1, &raw mut out, 1, std::ptr::null());
            libc::_exit(0);
        });
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if unsafe { libc::getppid() } == 1 {
                std::process::exit(0);
            }
        }
    });
}

#[cfg(test)]
mod exit_code_tests {
    #[test]
    fn a_signal_ended_msys_child_reads_as_128_plus_n_on_windows() {
        use super::own_code_on;
        assert_eq!(own_code_on(3840, true), 143, "SIGTERM: 15 << 8");
        assert_eq!(own_code_on(2304, true), 137, "SIGKILL: 9 << 8");
        assert_eq!(
            own_code_on(3, true),
            3,
            "an ordinary exit code is the program's own"
        );
        assert_eq!(own_code_on(0, true), 0);
        assert_eq!(own_code_on(65 << 8, true), 65 << 8, "no signal 65");
        assert_eq!(
            own_code_on(3840, false),
            3840,
            "Unix codes are never read this way"
        );
    }
}

#[cfg(all(test, unix))]
mod path_tests {
    use super::{Program, Spawn, host_spelling};
    use crate::file_access::{self as fa, Flavor, PathText, with_program_host};

    /// W1.1's red line: under the emulated Windows host, a `/`-spelled program path, working
    /// directory and path argument each reach their file — and a drive-spelled one too —
    /// while a text argument holding `a/b` arrives as it is written.
    #[test]
    fn a_program_path_reaches_its_file_under_the_emulated_host() {
        let dir = std::env::temp_dir().join(format!("loft_spawn_paths_{}", std::process::id()));
        let _ = fa::remove_dir_all(&dir);
        fa::create_dir_all(&dir).expect("scratch");
        fa::write(dir.join("in.txt"), "found").expect("write");
        let real = dir.to_str().expect("a text temp dir").to_string();
        let ran = with_program_host(Flavor::Windows, || {
            let sh = PathText::program("/bin/sh").expect("program path");
            let cwd = PathText::program(&real).expect("working directory");
            let arg = PathText::program(&format!("L:{real}/in.txt")).expect("argument");
            assert_eq!(
                sh.flavor(),
                Flavor::Windows,
                "the program's paths are Windows'"
            );
            Spawn::new(Program::path(&sh))
                .args(["-c", "cat \"$1\"; cat in.txt; printf '|%s' \"$2\"", "sh"])
                .arg_path(&arg)
                .arg("a/b")
                .cwd(&cwd)
                .run(b"")
                .expect("run")
        });
        let _ = fa::remove_dir_all(&dir);
        assert_eq!(String::from_utf8_lossy(&ran.stdout), "foundfound|a/b");
    }

    /// A path of the other platform is refused at `start`, never handed over as text.
    #[test]
    fn a_path_of_another_platform_is_refused() {
        let windows = PathText::parse("C:/x/y", Flavor::Windows);
        assert!(host_spelling(&windows).is_err());
        let r = Spawn::new(Program::search("true"))
            .arg_path(&windows)
            .start();
        assert!(r.is_err(), "a Windows path started a child on unix");
    }
}

#[cfg(unix)]
mod os {
    //! Process groups, the forwarding table, and (Linux) driver death.
    use super::Tree as Kind;
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

    /// How long a stopped child gets to end on `SIGTERM` before its group is killed.
    const GRACE: std::time::Duration = std::time::Duration::from_secs(2);

    pub(super) fn spawn(cmd: &mut Command, kind: Kind) -> std::io::Result<Child> {
        // A foreground child stays in loft's group: it is handed the terminal.
        if kind != Kind::Foreground {
            std::os::unix::process::CommandExt::process_group(cmd, 0);
        }
        #[cfg(target_os = "linux")]
        if kind != Kind::Detached {
            dies_with_driver(cmd);
            return spawn_on_keeper(cmd);
        }
        cmd.spawn()
    }

    /// Arm the child `cmd` will start to end when loft does, however loft ends (loft#1699):
    /// `PR_SET_PDEATHSIG` with `SIGTERM` rather than `SIGKILL`, so a child that is loft passes
    /// it on to its own owned groups — reaching its children that are not loft — before it
    /// ends.  The signal comes when the THREAD that spawned the child ends, which is why every
    /// such child is spawned by [`spawn_on_keeper`].
    #[cfg(target_os = "linux")]
    fn dies_with_driver(cmd: &mut Command) {
        use std::os::unix::process::CommandExt as _;
        let driver = std::process::id() as libc::pid_t;
        // SAFETY: the closure runs in the forked child before `exec` and calls only
        // async-signal-safe `prctl` / `getppid` / `_exit`; it touches no allocator or lock.
        unsafe {
            cmd.pre_exec(move || {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                // The driver may have died before `prctl` armed; a child already handed to
                // another parent must not start.
                if libc::getppid() != driver {
                    libc::_exit(0);
                }
                Ok(())
            });
        }
    }

    /// The group a child leads (none for a foreground child, which is in loft's), its id, and
    /// whether loft forwards signals to the group.
    pub(super) struct Tree {
        group: i32,
        pid: i32,
        forwarded: bool,
    }

    impl Tree {
        pub(super) fn adopt(child: &Child, kind: Kind) -> Tree {
            let pid = i32::try_from(child.id()).unwrap_or(0);
            let group = if kind == Kind::Foreground { 0 } else { pid };
            let forwarded = kind == Kind::Owned && group > 0 && forward_to(group);
            Tree {
                group,
                pid,
                forwarded,
            }
        }

        /// End everything in the group: `SIGTERM` first, so a child that is loft passes it on
        /// to the groups IT owns (a group of its own, which this signal does not reach), then
        /// `SIGKILL` for whatever is left once the child has ended or [`GRACE`] has passed.
        /// Only while the child is unreaped: its id names the group until then, and nothing
        /// else.  A foreground child has no group of its own: the signals reach it alone.
        pub(super) fn stop(&self, child: &mut Child) {
            if self.group <= 0 && self.pid <= 0 {
                return;
            }
            // SAFETY: `killpg` and `kill` deliver a signal and touch no memory; the child is
            // unreaped, so its id (and its group's) is still its own.
            unsafe {
                if self.group > 0 {
                    libc::killpg(self.group, libc::SIGTERM);
                } else {
                    libc::kill(self.pid, libc::SIGTERM);
                }
            }
            let until = std::time::Instant::now() + GRACE;
            while std::time::Instant::now() < until && !exited(child, false) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            if self.group > 0 {
                // SAFETY: as above; the child is still unreaped, so the group is still its own.
                unsafe {
                    libc::killpg(self.group, libc::SIGKILL);
                }
            }
        }

        pub(super) fn release(&mut self, _child: &Child) {
            if self.forwarded {
                forget(self.group);
                self.forwarded = false;
            }
        }

        /// The handle goes away while the child runs: the group stays in the table, held by
        /// the child, which is never reaped and so keeps its id until loft ends.
        #[allow(
            clippy::unused_self,
            reason = "one signature across platforms: the Windows twin keeps its job here"
        )]
        pub(super) fn keep(&mut self) {}
    }

    /// Has `child` exited?  Asked with `WNOWAIT`, so it stays unreaped and its id stays its
    /// own; `block` waits for the exit.  A child that cannot be asked about reads as ended.
    pub(super) fn exited(child: &mut Child, block: bool) -> bool {
        let pid: libc::id_t = child.id();
        let flags = libc::WEXITED | libc::WNOWAIT | if block { 0 } else { libc::WNOHANG };
        loop {
            // SAFETY: `waitid` writes one `siginfo_t` this frame owns; `WNOWAIT` reaps nothing.
            let (r, signo) = unsafe {
                let mut info: libc::siginfo_t = std::mem::zeroed();
                let r = libc::waitid(libc::P_PID, pid, &raw mut info, flags);
                (r, info.si_signo)
            };
            if r == 0 {
                // `WNOHANG` with nothing to report leaves the zeroed info as it was.
                return signo == libc::SIGCHLD;
            }
            if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                return true;
            }
        }
    }

    /// The owned groups a forwarded signal reaches: a fixed table the handler reads without a
    /// lock.  A group that finds it full is not forwarded to; a stop still reaches it.
    static GROUPS: [AtomicI32; 1024] = [const { AtomicI32::new(0) }; 1024];

    /// The signals loft passes on: the terminal's three, and the polite request to end.
    const FORWARDED: [libc::c_int; 4] = [libc::SIGINT, libc::SIGQUIT, libc::SIGHUP, libc::SIGTERM];

    /// What each of [`FORWARDED`] did before loft's forwarding took it over.
    static PREVIOUS: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
    static PREVIOUS_FLAGS: [AtomicI32; 4] = [const { AtomicI32::new(0) }; 4];

    fn forward_to(group: i32) -> bool {
        install_forwarding();
        GROUPS.iter().any(|slot| {
            slot.compare_exchange(0, group, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        })
    }

    fn forget(group: i32) {
        for slot in &GROUPS {
            let _ = slot.compare_exchange(group, 0, Ordering::SeqCst, Ordering::SeqCst);
        }
    }

    fn install_forwarding() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            for (i, &sig) in FORWARDED.iter().enumerate() {
                // SAFETY: `sigaction` reads and writes structs this frame owns; the handler it
                // installs is async-signal-safe (`forwarded`).
                unsafe {
                    let mut old: libc::sigaction = std::mem::zeroed();
                    libc::sigaction(sig, std::ptr::null(), &raw mut old);
                    // An ignored signal is ignored by the children too (the disposition
                    // survives `exec`): there is nothing to pass on.
                    if old.sa_sigaction == libc::SIG_IGN {
                        continue;
                    }
                    PREVIOUS[i].store(old.sa_sigaction, Ordering::SeqCst);
                    PREVIOUS_FLAGS[i].store(old.sa_flags, Ordering::SeqCst);
                    let mut act: libc::sigaction = std::mem::zeroed();
                    act.sa_sigaction = forwarded as *const () as libc::sighandler_t;
                    act.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
                    libc::sigemptyset(&raw mut act.sa_mask);
                    libc::sigaction(sig, &raw const act, std::ptr::null_mut());
                }
            }
        });
    }

    /// Pass `sig` to every owned group, then do what loft did with it before: end loft as the
    /// default action would, or call the handler that was there.
    extern "C" fn forwarded(sig: libc::c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
        for slot in &GROUPS {
            let g = slot.load(Ordering::Relaxed);
            if g > 0 {
                // SAFETY: `killpg` is async-signal-safe.
                unsafe {
                    libc::killpg(g, sig);
                }
            }
        }
        let Some(i) = FORWARDED.iter().position(|&s| s == sig) else {
            return;
        };
        let prev = PREVIOUS[i].load(Ordering::Relaxed);
        // SAFETY: `signal` and `raise` are async-signal-safe; a previous handler is called
        // the way it was installed (`SA_SIGINFO` or not).
        unsafe {
            if prev == libc::SIG_DFL {
                // The signal is blocked while this handler runs; it is delivered, with its
                // default action, the moment the handler returns.
                libc::signal(sig, libc::SIG_DFL);
                libc::raise(sig);
            } else if PREVIOUS_FLAGS[i].load(Ordering::Relaxed) & libc::SA_SIGINFO != 0 {
                let h: extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) =
                    std::mem::transmute(prev);
                h(sig, info, ctx);
            } else {
                let h: extern "C" fn(libc::c_int) = std::mem::transmute(prev);
                h(sig);
            }
        }
    }

    /// Linux delivers `PR_SET_PDEATHSIG` when the THREAD that spawned the child ends, not the
    /// process.  A child started from a worker thread that then finished would be ended with
    /// it, so every owned child is spawned by one thread that lives as long as loft.
    #[cfg(target_os = "linux")]
    #[allow(
        clippy::disallowed_methods,
        reason = "platform::process is the one home of Command::new (@PLN184 P6)"
    )]
    fn spawn_on_keeper(cmd: &mut Command) -> std::io::Result<Child> {
        use std::sync::mpsc;
        type Spawned = (Command, std::io::Result<Child>);
        type Job = (Command, mpsc::Sender<Spawned>);
        static KEEPER: std::sync::OnceLock<Option<std::sync::Mutex<mpsc::Sender<Job>>>> =
            std::sync::OnceLock::new();
        let keeper = KEEPER.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("loft-spawner".into())
                .spawn(move || {
                    for (mut cmd, back) in rx {
                        let child = cmd.spawn();
                        let _ = back.send((cmd, child));
                    }
                })
                .ok()
                .map(|_| std::sync::Mutex::new(tx))
        });
        let Some(keeper) = keeper else {
            return cmd.spawn();
        };
        // The command moves to the keeper and back, so the spawn can be started again;
        // `Command` cannot be cloned.
        let moved = std::mem::replace(cmd, Command::new(""));
        let (back, answer) = mpsc::channel();
        let sent = keeper
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send((moved, back));
        if let Err(mpsc::SendError((mut moved, _))) = sent {
            let child = moved.spawn();
            *cmd = moved;
            return child;
        }
        match answer.recv() {
            Ok((returned, child)) => {
                *cmd = returned;
                child
            }
            Err(_) => Err(std::io::Error::other("the spawning thread ended")),
        }
    }

    /// loft#1699 — a child armed by [`dies_with_driver`] must not outlive the thread that
    /// spawned it: here a thread that spawns and returns without waiting stands in for a
    /// driver that ends.  The child must end by `SIGTERM` at once, not run its five seconds;
    /// without the arming it is untouched and reads `exit 0` after five seconds.
    #[cfg(all(test, target_os = "linux"))]
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the arming itself is under test, below Spawn"
    )]
    fn a_child_dies_with_its_driver() {
        use std::os::unix::process::ExitStatusExt as _;
        let started = std::time::Instant::now();
        let mut child = std::thread::spawn(|| {
            let mut cmd = Command::new("sleep");
            cmd.arg("5");
            dies_with_driver(&mut cmd);
            cmd.spawn().expect("spawn sleep")
        })
        .join()
        .expect("spawning thread");
        let status = child.wait().expect("wait");
        assert_eq!(
            status.signal(),
            Some(libc::SIGTERM),
            "the child outlived its driver: {status}"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(4),
            "{:?}",
            started.elapsed()
        );
    }

    #[cfg(test)]
    mod tests {
        const PROBE: &str = "PLATFORM_PROCESS_FORWARD_PROBE";

        /// P2's terminal rule: a `SIGINT` sent to loft (as a `Ctrl-C` sends it to the
        /// terminal's foreground group, which an owned child has left) reaches the owned
        /// child's group.  The probe runs in a process of its own: the forwarded signal
        /// reaches every owned group in the process, and a sibling test's children are not
        /// this probe's to interrupt.
        #[test]
        fn an_interrupt_reaches_an_owned_child() {
            if std::env::var_os(PROBE).is_none() {
                let exe = std::env::current_exe().expect("the test binary");
                let out = super::super::harness_command(exe)
                    .args([
                        "--exact",
                        "platform::process::os::tests::an_interrupt_reaches_an_owned_child",
                        "--nocapture",
                    ])
                    .env(PROBE, "1")
                    .output()
                    .expect("run the probe");
                assert!(
                    out.status.success()
                        && String::from_utf8_lossy(&out.stdout).contains("1 passed"),
                    "{}\n{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
                return;
            }
            let dir = std::env::temp_dir().join(format!("loft_forward_{}", std::process::id()));
            let _ = crate::file_access::remove_dir_all(&dir);
            crate::file_access::create_dir_all(&dir).expect("scratch");
            let marker = dir.join("got_int");
            // A child that records the interrupt it receives, then ends.
            let script = format!(
                "trap 'echo int > {}; exit 0' INT; echo ready; while :; do sleep 0.05; done",
                marker.display()
            );
            let mut p = super::super::Spawn::new(super::super::Program::search("sh"))
                .args(["-c", &script])
                .stdout(std::process::Stdio::piped())
                .start()
                .expect("spawn sh");
            let mut out = p.take_stdout().expect("stdout");
            let mut ready = [0u8; 6];
            std::io::Read::read_exact(&mut out, &mut ready).expect("ready");
            // Through the handler loft installs, as a terminal's interrupt would arrive —
            // with the previous action swapped for one that keeps this test process alive.
            super::PREVIOUS[0].store(
                ignore as *const () as usize,
                std::sync::atomic::Ordering::SeqCst,
            );
            super::PREVIOUS_FLAGS[0].store(0, std::sync::atomic::Ordering::SeqCst);
            // SAFETY: `raise` delivers SIGINT to this process, whose handler is `forwarded`.
            unsafe {
                libc::raise(libc::SIGINT);
            }
            let arrived = p.wait(std::time::Duration::from_secs(5));
            let got = crate::file_access::exists(&marker);
            let _ = p.stop_tree();
            let _ = crate::file_access::remove_dir_all(&dir);
            assert!(arrived && got, "the owned child never saw the interrupt");
        }

        extern "C" fn ignore(_: libc::c_int) {}
    }
}

#[cfg(windows)]
mod os {
    //! The Job Object (@PLN184 P4).  An owned child is assigned to a job of its own, created
    //! with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: everything the child starts joins the job,
    //! [`Tree::stop`] ends the whole job, and when loft ends — however it ends — the OS closes
    //! the job's last handle and ends what is left.  The five Win32 calls are declared by hand:
    //! the dependency rule takes them over a crate.
    //!
    //! **Spawned, then assigned — not `CREATE_SUSPENDED`.**  Between `CreateProcess` returning
    //! and the assignment, the child runs, and a grandchild it started in that window would
    //! not be in the job.  Starting it suspended closes the window, but resuming it needs the
    //! primary thread's handle, which `std::process::Child` does not expose, so it would cost an
    //! undocumented `NtResumeProcess` from `ntdll` — a sixth call.  The window is the
    //! assignment's few microseconds against the child's own start-up (the loader, its DLLs,
    //! the runtime, `main`) before it can start anything, which is far longer for every child
    //! loft starts (rustc, cargo, a game, loft itself).  The windows-latest cell
    //! `a_grandchild_started_at_once_is_in_the_tree` measures it.
    //!
    //! A job's grandchild started `Tree::Detached` breaks away (`CREATE_BREAKAWAY_FROM_JOB`,
    //! which every job here allows), as a detached child leaves its parent's group on unix.
    use super::Tree as Kind;
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle as _;
    use std::os::windows::process::CommandExt as _;
    use std::process::{Child, Command};

    type Handle = *mut c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(job: Handle, class: i32, info: *const c_void, len: u32) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    /// `JOBOBJECT_BASIC_LIMIT_INFORMATION`.
    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        per_process_user_time: i64,
        per_job_user_time: i64,
        limit_flags: u32,
        min_working_set: usize,
        max_working_set: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    /// `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`.
    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io_counters: [u64; 6],
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    /// `JobObjectExtendedLimitInformation`.
    const EXTENDED_LIMIT_INFORMATION: i32 = 9;
    const KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const BREAKAWAY_OK: u32 = 0x0800;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    const ERROR_ACCESS_DENIED: i32 = 5;
    /// The exit code a stopped tree reports: `128 + SIGTERM`, what a stop reads as on unix.
    const STOPPED: u32 = 143;

    /// A job handle.  Closing it ends whatever the job still holds.
    struct Job(Handle);

    // SAFETY: a kernel handle is a process-wide value; every call on it is thread-safe.
    unsafe impl Send for Job {}
    // SAFETY: as above.
    unsafe impl Sync for Job {}

    impl Job {
        /// A job that ends its processes when its last handle closes, and lets a detached
        /// grandchild break away.
        fn new() -> Option<Job> {
            // SAFETY: the calls take a null name and attributes, and a limits struct this
            // frame owns, of the size passed.
            unsafe {
                let h = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
                if h.is_null() {
                    return None;
                }
                let job = Job(h);
                let mut limits = ExtendedLimits::default();
                limits.basic.limit_flags = KILL_ON_JOB_CLOSE | BREAKAWAY_OK;
                let size = u32::try_from(std::mem::size_of::<ExtendedLimits>()).ok()?;
                let ok = SetInformationJobObject(
                    job.0,
                    EXTENDED_LIMIT_INFORMATION,
                    (&raw const limits).cast(),
                    size,
                );
                (ok != 0).then_some(job)
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle is this job's own, closed once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub(super) fn spawn(cmd: &mut Command, kind: Kind) -> std::io::Result<Child> {
        // Owned and foreground alike stay in the console: the job is what owns them.
        if kind != Kind::Detached {
            return cmd.spawn();
        }
        // Detached: out of loft's job, and out of the console's Ctrl-C group.
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        match cmd.spawn() {
            // loft itself runs in a job that forbids breaking away (a CI runner's): the
            // target stays in that job, as it stays in a session on unix.
            Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => {
                cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
                cmd.spawn()
            }
            r => r,
        }
    }

    /// The job an owned child was assigned to; `None` for a detached child, or when the OS
    /// refused a job (the stop then walks the tree by parent link instead).
    pub(super) struct Tree {
        job: Option<Job>,
    }

    impl Tree {
        pub(super) fn adopt(child: &Child, kind: Kind) -> Tree {
            let job = (kind != Kind::Detached)
                .then(Job::new)
                .flatten()
                .filter(|job| {
                    // SAFETY: both handles are live: the job's own, and the child's, which
                    // `Child` holds until it is dropped.
                    unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle()) != 0 }
                });
            Tree { job }
        }

        #[allow(
            clippy::disallowed_methods,
            reason = "platform::process is the one home of Command::new (@PLN184 P6)"
        )]
        pub(super) fn stop(&self, child: &mut Child) {
            if let Some(job) = &self.job {
                // SAFETY: the job's own handle.
                unsafe {
                    TerminateJobObject(job.0, STOPPED);
                }
                return;
            }
            // No job: walk the tree by parent link.  `taskkill /T` needs the child ALIVE to
            // walk from, so it runs before the kill.
            let _ = Command::new("taskkill")
                .args(["/T", "/F", "/PID", &child.id().to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }

        /// The job outlives the reap: it is what still reaches the child's stragglers.
        #[allow(clippy::unused_self, reason = "one signature across platforms")]
        pub(super) fn release(&mut self, _child: &Child) {}

        /// The handle goes away: the job stays open until loft ends, so what the child
        /// started still ends with loft, and not before.
        pub(super) fn keep(&mut self) {
            if let Some(job) = self.job.take() {
                std::mem::forget(job);
            }
        }
    }

    pub(super) fn exited(child: &mut Child, block: bool) -> bool {
        if block {
            let _ = child.wait();
            return true;
        }
        !matches!(child.try_wait(), Ok(None))
    }
}

#[cfg(not(any(unix, windows)))]
mod os {
    use super::Tree as Kind;
    use std::process::{Child, Command};

    pub(super) fn spawn(cmd: &mut Command, _kind: Kind) -> std::io::Result<Child> {
        cmd.spawn()
    }

    pub(super) struct Tree;

    #[allow(clippy::unused_self, reason = "one signature across platforms")]
    impl Tree {
        pub(super) fn adopt(_child: &Child, _kind: Kind) -> Tree {
            Tree
        }
        pub(super) fn stop(&self, _child: &mut Child) {}
        pub(super) fn release(&mut self, _child: &Child) {}
        pub(super) fn keep(&mut self) {}
    }

    pub(super) fn exited(child: &mut Child, block: bool) -> bool {
        if block {
            let _ = child.wait();
            return true;
        }
        !matches!(child.try_wait(), Ok(None))
    }
}
