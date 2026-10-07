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
//! hands it the terminal (its input is a pipe or nothing).
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
    /// aimed at loft's own tree, and not loft's end, reaches it.  Its own stop still reaches
    /// what it started.
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
        let child = os::spawn(&mut self.cmd, self.tree)?;
        Ok(Running::new(child, self.tree))
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

#[cfg(unix)]
mod os {
    //! Process groups, the forwarding table, and (Linux) driver death.
    use super::Tree as Kind;
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

    /// How long a stopped child gets to end on `SIGTERM` before its group is killed.
    const GRACE: std::time::Duration = std::time::Duration::from_secs(2);

    pub(super) fn spawn(cmd: &mut Command, kind: Kind) -> std::io::Result<Child> {
        std::os::unix::process::CommandExt::process_group(cmd, 0);
        #[cfg(target_os = "linux")]
        if kind == Kind::Owned {
            // `SIGTERM` rather than `SIGKILL`, so a child that is loft passes it on to its own
            // owned groups — reaching its children that are not loft — before it ends.
            crate::platform::dies_with_driver(cmd, true);
            return spawn_on_keeper(cmd);
        }
        let _ = kind;
        cmd.spawn()
    }

    /// The group a child leads, and whether loft forwards signals to it.
    pub(super) struct Tree {
        group: i32,
        forwarded: bool,
    }

    impl Tree {
        pub(super) fn adopt(child: &Child, kind: Kind) -> Tree {
            let group = i32::try_from(child.id()).unwrap_or(0);
            let forwarded = kind == Kind::Owned && group > 0 && forward_to(group);
            Tree { group, forwarded }
        }

        /// End everything in the group: `SIGTERM` first, so a child that is loft passes it on
        /// to the groups IT owns (a group of its own, which this signal does not reach), then
        /// `SIGKILL` for whatever is left once the child has ended or [`GRACE`] has passed.
        /// Only while the child is unreaped: its id names the group until then, and nothing
        /// else.
        pub(super) fn stop(&self, child: &mut Child) {
            if self.group <= 0 {
                return;
            }
            // SAFETY: `killpg` delivers a signal and touches no memory.
            unsafe {
                libc::killpg(self.group, libc::SIGTERM);
            }
            let until = std::time::Instant::now() + GRACE;
            while std::time::Instant::now() < until && !exited(child, false) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            // SAFETY: as above; the child is still unreaped, so the group is still its own.
            unsafe {
                libc::killpg(self.group, libc::SIGKILL);
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
    fn spawn_on_keeper(cmd: &mut Command) -> std::io::Result<Child> {
        use std::sync::mpsc;
        type Job = (Command, mpsc::Sender<std::io::Result<Child>>);
        static KEEPER: std::sync::OnceLock<Option<std::sync::Mutex<mpsc::Sender<Job>>>> =
            std::sync::OnceLock::new();
        let keeper = KEEPER.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("loft-spawner".into())
                .spawn(move || {
                    for (mut cmd, back) in rx {
                        let _ = back.send(cmd.spawn());
                    }
                })
                .ok()
                .map(|_| std::sync::Mutex::new(tx))
        });
        let Some(keeper) = keeper else {
            return cmd.spawn();
        };
        // The command moves to the keeper and back; `Command` cannot be cloned.
        let moved = std::mem::replace(cmd, Command::new(""));
        let (back, answer) = mpsc::channel();
        let sent = keeper
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send((moved, back));
        if let Err(mpsc::SendError((mut moved, _))) = sent {
            return moved.spawn();
        }
        answer
            .recv()
            .unwrap_or_else(|_| Err(std::io::Error::other("the spawning thread ended")))
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
                let out = std::process::Command::new(exe)
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
    //! Windows has no process group: until the Job Object (P4), a stop walks the tree by
    //! parent link, as the repl's game stop did.
    use super::Tree as Kind;
    use std::process::{Child, Command};

    pub(super) fn spawn(cmd: &mut Command, _kind: Kind) -> std::io::Result<Child> {
        cmd.spawn()
    }

    pub(super) struct Tree;

    #[allow(clippy::unused_self, reason = "until P4 holds a job here")]
    impl Tree {
        pub(super) fn adopt(_child: &Child, _kind: Kind) -> Tree {
            Tree
        }
        /// `taskkill /T` needs the child ALIVE to walk from, so it runs before the kill.
        pub(super) fn stop(&self, child: &mut Child) {
            let _ = Command::new("taskkill")
                .args(["/T", "/F", "/PID", &child.id().to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
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
