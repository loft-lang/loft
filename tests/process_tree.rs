// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN184 Track P0 — a process loft starts is stopped together with everything it started,
//! on every platform, unless it was started to outlive loft.
//!
//! The fixture is a tree three deep: this test starts a CHILD, and the child starts a
//! GRANDCHILD.  The grandchild is this same test binary run as `beat_role`: it writes a
//! counter to a file every 50 ms until a stop file appears, so "is it still running" is a
//! question about a file, the same on every platform, with no pid to recycle.  Every cell
//! first proves the grandchild beats — a cell whose fixture never started it would pass
//! vacuously — and then asks whether it still beats after the stop.
//!
//! The cells:
//! - **tree stop**: the child is `loft` running a program whose `process` library call
//!   started the grandchild; the test starts and stops the child through
//!   `platform::process` (`Tree::Owned`, `stop_tree`), as a site that owns a tree does.  The
//!   grandchild must stop with it.
//! - **driver death, a loft program**: the same child, but the test KILLS it.  Its program's
//!   grandchild must not outlive it.
//! - **driver death, a Rust site**: the child is this binary as `driver_role`, starting the
//!   grandchild `Tree::Owned`, as the build sites start `rustc` and `cargo`; the test kills
//!   the child.
//! - **handover**: the same driver starts the grandchild `Tree::Detached`, to OUTLIVE it
//!   (the engine host's hot-swap target); the grandchild must still beat after the driver is
//!   killed.
//!
//! - **a grandchild started at once**: the child is this binary as `spawner_role`, an
//!   ordinary program that starts the grandchild the moment it runs; the test starts it
//!   owned and stops it, five times.  On Windows this is the question P4 decided without
//!   `CREATE_SUSPENDED`: a child is assigned to its job just after it starts, and a
//!   grandchild started before that would escape the job.
//!
//! The killing is done from outside, with a plain process handle: the test stands in for
//! what ends a driver without asking it (a timeout, the OOM killer, a harness reaping it).

use loft::file_access as fa;
use loft::platform::process::{Program, Spawn, Tree};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const ROLE: &str = "PROCESS_TREE_ROLE";
const DIR: &str = "PROCESS_TREE_DIR";
const EXE: &str = "PROCESS_TREE_EXE";

fn exe() -> PathBuf {
    std::env::current_exe().expect("the test binary")
}

/// A fresh directory for one cell's pid, beat and stop files.
fn scratch(cell: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loft_process_tree_{}_{cell}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// The command line that runs this binary as `role`.
fn role_args(role: &str) -> [String; 4] {
    [
        role.to_string(),
        "--exact".to_string(),
        "--nocapture".to_string(),
        "--test-threads=1".to_string(),
    ]
}

fn beat_count(dir: &Path) -> Option<u64> {
    fa::read_to_string(dir.join("beat"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Does the grandchild still beat: its counter moves within a second.
fn beats(dir: &Path) -> bool {
    let first = beat_count(dir);
    let until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
        if beat_count(dir) != first && beat_count(dir).is_some() {
            return true;
        }
    }
    false
}

/// Wait until the grandchild beats, failing the cell when it never starts.  `child_alive`
/// asks whether the child still runs: one that ended first never started it.
fn wait_for_beat(dir: &Path, mut child_alive: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(60);
    while Instant::now() < until {
        if beat_count(dir).is_some() && beats(dir) {
            return;
        }
        if !child_alive() {
            panic!("the child ended before the grandchild started beating");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("the grandchild never started beating in {}", dir.display());
}

/// Has the grandchild stopped: no beat for a second, asked for up to five.
fn stopped(dir: &Path) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if !beats(dir) {
            return true;
        }
    }
    false
}

/// End a grandchild a failed cell left behind, and the cell's files.
fn release(dir: &Path) {
    let _ = fa::write(dir.join("stop"), "");
    std::thread::sleep(Duration::from_millis(200));
    let _ = fa::remove_dir_all(dir);
}

/// The program the loft child runs: its `process` library call starts the grandchild and
/// blocks in it.
const PROGRAM: &str = r#"use process::*;
fn main() {
  exe = env_variable("PROCESS_TREE_EXE");
  c: Command = "{exe} beat_role --exact --nocapture --test-threads=1";
  r = c.run();
  println("the grandchild ended: {r.code}");
}
"#;

/// The arguments and environment of `loft` running [`PROGRAM`], its grandchild told where
/// to beat.
fn loft_run(
    dir: &Path,
) -> (
    Vec<std::ffi::OsString>,
    Vec<(&'static str, std::ffi::OsString)>,
) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = dir.join("tree.loft");
    fa::write(&script, PROGRAM).expect("write the program");
    let args = vec![
        "--interpret".into(),
        "--lib".into(),
        root.join("lib").into_os_string(),
        script.into_os_string(),
    ];
    let env = vec![
        ("LOFT_TIMEOUT", "120".into()),
        (ROLE, "beat".into()),
        (DIR, dir.as_os_str().to_os_string()),
        (EXE, exe().into_os_string()),
    ];
    (args, env)
}

/// [`loft_run`] as a plain process, for a cell that kills it from outside.
fn loft_child(dir: &Path) -> Command {
    let (args, env) = loft_run(dir);
    let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
    cmd.args(args)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd
}

/// This binary as a driver that starts the grandchild as `role` says, then waits.
fn driver_child(dir: &Path, role: &str) -> Command {
    let mut cmd = loft::platform::process::harness_command(exe());
    cmd.args(role_args("driver_role"))
        .env(ROLE, role)
        .env(DIR, dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd
}

/// The grandchild: beats until the stop file appears, two minutes at most.
#[test]
fn beat_role() {
    if std::env::var(ROLE).as_deref() != Ok("beat") {
        return;
    }
    let dir = PathBuf::from(std::env::var_os(DIR).expect("PROCESS_TREE_DIR"));
    let _ = fa::write(dir.join("pid"), std::process::id().to_string());
    let until = Instant::now() + Duration::from_secs(120);
    let mut n = 0u64;
    while Instant::now() < until && !fa::exists(dir.join("stop")) {
        n += 1;
        let _ = fa::write(dir.join("beat"), n.to_string());
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The driver: starts the grandchild the way `PROCESS_TREE_ROLE` names, then waits to be
/// killed.
#[test]
fn driver_role() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    if !role.starts_with("driver-") {
        return;
    }
    let tree = match role.as_str() {
        // As the build sites start `rustc` and `cargo`.
        "driver-owned" => Tree::Owned,
        // As the engine host starts its hot-swap target.
        "driver-detached" => Tree::Detached,
        other => panic!("unknown role {other}"),
    };
    // The grandchild inherits PROCESS_TREE_DIR, and beats there.
    let mut grandchild = Spawn::new(Program::os(exe()))
        .args(role_args("beat_role"))
        .env(ROLE, "beat")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .tree(tree)
        .start()
        .expect("spawn the grandchild");
    let _ = grandchild.finish();
}

/// An ordinary program that starts the grandchild as the first thing it does, with the
/// plain standard library — no `platform::process` — and waits for it.
#[test]
fn spawner_role() {
    if std::env::var(ROLE).as_deref() != Ok("spawner") {
        return;
    }
    let mut grandchild = loft::platform::process::harness_command(exe())
        .args(role_args("beat_role"))
        .env(ROLE, "beat")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the grandchild");
    let _ = grandchild.wait();
}

#[test]
fn a_grandchild_started_at_once_is_in_the_tree() {
    for round in 0..5 {
        let dir = scratch(&format!("at_once_{round}"));
        let mut child = Spawn::new(Program::os(exe()))
            .args(role_args("spawner_role"))
            .env(ROLE, "spawner")
            .env(DIR, &dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .start()
            .expect("spawn the spawner");
        wait_for_beat(&dir, || child.alive());
        let _ = child.stop_tree();
        let gone = stopped(&dir);
        release(&dir);
        assert!(
            gone,
            "round {round}: a grandchild started at once escaped the tree"
        );
    }
}

#[test]
fn a_stopped_child_takes_its_programs_children_with_it() {
    let dir = scratch("tree_stop");
    let (args, env) = loft_run(&dir);
    let mut spawn = Spawn::new(Program::os(env!("CARGO_BIN_EXE_loft"))).args(args);
    for (k, v) in env {
        spawn = spawn.env(k, v);
    }
    let mut child = spawn
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .start()
        .expect("spawn loft");
    wait_for_beat(&dir, || child.alive());
    let _ = child.stop_tree();
    let gone = stopped(&dir);
    release(&dir);
    assert!(
        gone,
        "tree stop: the grandchild outlived its stopped parent"
    );
}

#[test]
fn a_killed_loft_takes_its_programs_children_with_it() {
    let dir = scratch("driver_death_loft");
    let mut child = loft_child(&dir).spawn().expect("spawn loft");
    wait_for_beat(&dir, || matches!(child.try_wait(), Ok(None)));
    let _ = child.kill();
    let _ = child.wait();
    let gone = stopped(&dir);
    release(&dir);
    assert!(
        gone,
        "driver death: a loft program's child outlived the killed loft"
    );
}

#[test]
fn a_killed_driver_takes_its_build_child_with_it() {
    let dir = scratch("driver_death_rust");
    let mut child = driver_child(&dir, "driver-owned")
        .spawn()
        .expect("spawn the driver");
    wait_for_beat(&dir, || matches!(child.try_wait(), Ok(None)));
    let _ = child.kill();
    let _ = child.wait();
    let gone = stopped(&dir);
    release(&dir);
    assert!(
        gone,
        "driver death: a build child outlived its killed driver"
    );
}

#[test]
fn a_handover_target_outlives_its_driver() {
    let dir = scratch("handover");
    let mut child = driver_child(&dir, "driver-detached")
        .spawn()
        .expect("spawn the driver");
    wait_for_beat(&dir, || matches!(child.try_wait(), Ok(None)));
    let _ = child.kill();
    let _ = child.wait();
    // Give a wrongly owned target the time a stop takes before asking.
    std::thread::sleep(Duration::from_millis(500));
    let alive = beats(&dir);
    release(&dir);
    assert!(alive, "handover: the detached target died with its driver");
}
