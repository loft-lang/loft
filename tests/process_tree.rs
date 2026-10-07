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
//!   started the grandchild; the test stops the child the way a site that owns a tree does.
//!   The grandchild must stop with it.
//! - **driver death, a loft program**: the same child, but the test KILLS it.  Its program's
//!   grandchild must not outlive it.
//! - **driver death, a Rust site**: the child is this binary as `driver_role`, starting the
//!   grandchild as the build sites start `rustc` and `cargo`; the test kills the child.
//! - **handover**: the same driver starts the grandchild to OUTLIVE it (the engine host's
//!   hot-swap target); the grandchild must still beat after the driver is killed.

use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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

/// Wait until the grandchild beats, failing the cell when it never starts.
fn wait_for_beat(dir: &Path, child: &mut Child) {
    let until = Instant::now() + Duration::from_secs(60);
    while Instant::now() < until {
        if beat_count(dir).is_some() && beats(dir) {
            return;
        }
        if let Ok(Some(status)) = child.try_wait() {
            panic!("the child ended ({status}) before the grandchild started beating");
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

/// `loft` running [`PROGRAM`], its grandchild told where to beat.
fn loft_child(dir: &Path) -> Command {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = dir.join("tree.loft");
    fa::write(&script, PROGRAM).expect("write the program");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("--interpret")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&script)
        .env("LOFT_TIMEOUT", "120")
        .env(ROLE, "beat")
        .env(DIR, dir)
        .env(EXE, exe())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd
}

/// This binary as a driver that starts the grandchild as `role` says, then waits.
fn driver_child(dir: &Path, role: &str) -> Command {
    let mut cmd = Command::new(exe());
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
    // The grandchild inherits PROCESS_TREE_DIR, and beats there.
    let mut cmd = Command::new(exe());
    cmd.args(role_args("beat_role"))
        .env(ROLE, "beat")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match role.as_str() {
        // As the build sites start `rustc` and `cargo` today.
        "driver-owned" => loft::platform::dies_with_driver(&mut cmd, false),
        // As the engine host starts its hot-swap target today.
        "driver-detached" => {
            #[cfg(unix)]
            std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        }
        other => panic!("unknown role {other}"),
    }
    let mut grandchild = cmd.spawn().expect("spawn the grandchild");
    let _ = grandchild.wait();
}

/// Stop `child` the way a site that owns a tree stops it today: the repl's game server
/// signals the process group the launch made; elsewhere the handle is all there is.
fn stop_tree_today(child: &mut Child) {
    #[cfg(unix)]
    // SAFETY: `killpg` on the group this test created for `child`, which is not yet reaped.
    unsafe {
        libc::killpg(child.id() as i32, libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn a_stopped_child_takes_its_programs_children_with_it() {
    let dir = scratch("tree_stop");
    let mut cmd = loft_child(&dir);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn().expect("spawn loft");
    wait_for_beat(&dir, &mut child);
    stop_tree_today(&mut child);
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
    wait_for_beat(&dir, &mut child);
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
    wait_for_beat(&dir, &mut child);
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
    wait_for_beat(&dir, &mut child);
    let _ = child.kill();
    let _ = child.wait();
    // Give a wrongly owned target the time a stop takes before asking.
    std::thread::sleep(Duration::from_millis(500));
    let alive = beats(&dir);
    release(&dir);
    assert!(alive, "handover: the detached target died with its driver");
}
