// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I78 — Live-reload dispatch: the background rebuild

//! The serve host's background rebuild: spawn the loft driver's `--check --native` as a
//! child, snapshot the source at request time, and requeue when it changed mid-build.  Only
//! process orchestration, so it sits in the runtime a compiled program links (@PLN191);
//! `live_dispatch` re-exports it.

use std::cell::UnsafeCell;

use crate::database::Stores;
use crate::keys::DbRef;

// ── @PLN18 08-S4 — the background rebuild ──────────────────────────────────
//
// The serve host compiles the full project while the old build keeps
// serving.  The build pipeline is the loft DRIVER's own `--check --native`
// (it compiles into the content-hash cache and prints the artifact path on
// the ok line); this module only orchestrates: spawn it as a background
// child, snapshot the loft source at request time, and on completion
// REQUEUE when the source changed mid-build (the staleness contract).
// Stdout/stderr go to temp FILES, not pipes — a failed rustc prints more
// than a pipe buffer holds, and a blocked child would hang the poll.

/// Rebuild status codes (the loft-visible `rebuild_status()` contract).
#[cfg(not(target_arch = "wasm32"))]
const REBUILD_IDLE: i64 = 0;
#[cfg(not(target_arch = "wasm32"))]
const REBUILD_BUILDING: i64 = 1;
#[cfg(not(target_arch = "wasm32"))]
const REBUILD_READY: i64 = 2;
#[cfg(not(target_arch = "wasm32"))]
const REBUILD_FAILED: i64 = 3;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
enum Rebuild {
    #[default]
    Idle,
    Building {
        child: crate::platform::process::Running,
        /// The loft source bytes at spawn — completion compares against the
        /// CURRENT file; a mismatch means the artifact is already stale.
        snapshot: Vec<u8>,
        out_path: std::path::PathBuf,
        err_path: std::path::PathBuf,
    },
    Ready {
        artifact: String,
    },
    Failed,
}

/// `@PLN226` — a program's background rebuild: the build in flight or its outcome, the builds
/// numbered so far, and why the last one failed (for the editor that asked, `D!:rebuild?`).
/// The program's own, in its engine context (`engine_host::EngineCtl::rebuild`).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub(crate) struct RebuildCx {
    state: Rebuild,
    seq: u32,
    failure: String,
    /// The tier of the rebuild in flight or ready: the shipped one (`--ship`) for `rebuild
    /// fast` and in a build swapped in at full speed (`LOFT_LIVE_SHIP=1`).
    ship: bool,
}

/// What an interpreted run tells the rebuild about itself: the driver to build with, the
/// program's source and its library directories.  A compiled build gets the same from the
/// driver as `LOFT_LIVE_DRIVER`, `LOFT_LIVE_SRC` and `LOFT_LIVE_LIBS`, which win.
#[cfg(not(target_arch = "wasm32"))]
struct LiveHost {
    driver: String,
    src: String,
    libs: String,
}

#[cfg(not(target_arch = "wasm32"))]
static LIVE_HOST: std::sync::OnceLock<LiveHost> = std::sync::OnceLock::new();

/// Called by the driver before it interprets `src`, so an interpreted serve host rebuilds
/// and swaps as a compiled one does.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_interp_host(driver: &str, src: &str, lib_dirs: &[String]) {
    let _ = LIVE_HOST.set(LiveHost {
        driver: driver.to_string(),
        src: src.to_string(),
        libs: lib_dirs.join(":"),
    });
}

/// One of the live variables: the environment's, else the interpreted run's own.
#[cfg(not(target_arch = "wasm32"))]
fn live_var(name: &str) -> Option<String> {
    if let Ok(v) = std::env::var(name) {
        return Some(v);
    }
    let host = LIVE_HOST.get()?;
    let v = match name {
        "LOFT_LIVE_DRIVER" => &host.driver,
        "LOFT_LIVE_SRC" => &host.src,
        "LOFT_LIVE_LIBS" => &host.libs,
        _ => return None,
    };
    Some(v.clone())
}

/// The successor a swap of an INTERPRETED host starts: the same program, interpreted again —
/// its driver, `--interpret`, its library directories and its source — so a development
/// session keeps its debugger, its flips and Apply across a rebuild (`@C170`: those live in the
/// interpreter).  The rebuild's C build stays the check that the edit compiles.  `None` for a
/// host the driver did not interpret, which swaps into the rebuilt binary.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn interpreted_successor() -> Option<Vec<String>> {
    let host = LIVE_HOST.get()?;
    let mut argv = vec![host.driver.clone(), "--interpret".to_string()];
    for d in host.libs.split(':').filter(|d| !d.is_empty()) {
        argv.push("--lib".to_string());
        argv.push(d.to_string());
    }
    argv.push(host.src.clone());
    Some(argv)
}

/// The live variables a swapped-in build is started with, so it can rebuild in turn: those
/// the environment it inherits does not hold already.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn live_env_for_successor() -> Vec<(&'static str, String)> {
    ["LOFT_LIVE_DRIVER", "LOFT_LIVE_SRC", "LOFT_LIVE_LIBS"]
        .into_iter()
        .filter(|n| std::env::var_os(n).is_none())
        .filter_map(|n| live_var(n).map(|v| (n, v)))
        .collect()
}

/// Spawn one background `--check --native` build of `src` via the driver.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_build(
    seq: &mut u32,
    ship: bool,
    layout: Option<&str>,
    driver: &str,
    src: &str,
) -> Result<Rebuild, String> {
    // The error starts with the path: "cannot read <src>: <why>".
    let snapshot = crate::file_access::read(src).map_err(|e| format!("cannot read {e}"))?;
    *seq += 1;
    let seq = *seq;
    let base = std::env::temp_dir().join(format!("loft_rebuild_{}_{seq}", std::process::id()));
    let out_path = crate::file_access::with_extension(&base, "out");
    let err_path = crate::file_access::with_extension(&base, "err");
    let out = crate::file_access::create(&out_path).map_err(|e| e.to_string())?;
    let err = crate::file_access::create(&err_path).map_err(|e| e.to_string())?;
    use crate::platform::process::{Program, Spawn};
    let mut cmd = Spawn::new(Program::os(driver))
        .arg("--no-warnings")
        .arg("--check")
        .arg("--native");
    if ship {
        cmd.push_arg("--ship");
    }
    if let Some(libs) = live_var("LOFT_LIVE_LIBS") {
        for d in libs.split(':').filter(|s| !s.is_empty()) {
            cmd.push_arg("--lib").push_arg(d);
        }
    }
    // @PLN196 step 8 — the build of the next instance is checked against the running world:
    // a world it cannot take over fails the build here, never the swap.
    if let Some(layout) = layout {
        let world = crate::file_access::with_extension(&base, "world.dschema");
        crate::file_access::write(&world, layout).map_err(|e| e.to_string())?;
        cmd = cmd.env("LOFT_CHECK_WORLD", &world);
    }
    // A build legitimately takes minutes — never under the run watchdog.
    let child = cmd
        .arg(src)
        // Ask the driver for the machine form of its ok line: "ok <src> <artifact>".
        // Without it the driver answers a person, and prints `ok` alone.
        .env("LOFT_CHECK_ARTIFACT", "1")
        .env_remove("LOFT_TIMEOUT")
        .stdout(out.into())
        .stderr(err.into())
        .stdin(std::process::Stdio::null())
        .start()
        .map_err(|e| format!("cannot spawn {driver}: {e}"))?;
    Ok(Rebuild::Building {
        child,
        snapshot,
        out_path,
        err_path,
    })
}

/// Start a background rebuild of the program this binary serves.  True =
/// building (idempotent while one runs); false = the host lacks the live
/// env (run through the loft driver) or the spawn failed — warned, never
/// a halt.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_start(rc: &mut RebuildCx, layout: Option<&str>) -> bool {
    rebuild_start_tier(
        rc,
        layout,
        std::env::var("LOFT_LIVE_SHIP").is_ok_and(|v| v == "1"),
    )
}

/// Start a rebuild, of the shipped tier (`--ship`, the speed the game ships at) when `ship`,
/// else of the semantics tier the check reads.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_start_tier(rc: &mut RebuildCx, layout: Option<&str>, ship: bool) -> bool {
    let (Some(driver), Some(src)) = (live_var("LOFT_LIVE_DRIVER"), live_var("LOFT_LIVE_SRC"))
    else {
        eprintln!(
            "loft-live: rebuild needs LOFT_LIVE_DRIVER/LOFT_LIVE_SRC (run through the loft driver)"
        );
        return false;
    };
    {
        let r = &mut rc.state;
        if matches!(*r, Rebuild::Building { .. }) && rc.ship == ship {
            return true; // idempotent: one build at a time
        }
        rc.ship = ship;
        match spawn_build(&mut rc.seq, ship, layout, &driver, &src) {
            Ok(b) => {
                eprintln!("loft-live: rebuild started ({src})");
                *r = b;
                true
            }
            Err(msg) => {
                eprintln!("loft-live: rebuild failed to start — {msg}");
                *r = Rebuild::Failed;
                false
            }
        }
    }
}

/// Poll the rebuild: 0 idle, 1 building, 2 ready, 3 failed.  Completion
/// with a source that changed mid-build REQUEUES automatically (status
/// stays 1) — the artifact must correspond to the source as of the check.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_status(rc: &mut RebuildCx, layout: Option<&str>) -> i64 {
    {
        let r = &mut rc.state;
        let Rebuild::Building {
            child,
            snapshot,
            out_path,
            err_path,
        } = &mut *r
        else {
            return match &*r {
                Rebuild::Idle => REBUILD_IDLE,
                Rebuild::Ready { .. } => REBUILD_READY,
                _ => REBUILD_FAILED,
            };
        };
        if child.alive() {
            return REBUILD_BUILDING;
        }
        let status = match child.finish() {
            Ok(st) => st,
            Err(e) => {
                eprintln!("loft-live: rebuild wait failed — {e}");
                *r = Rebuild::Failed;
                return REBUILD_FAILED;
            }
        };
        let src = live_var("LOFT_LIVE_SRC").unwrap_or_default();
        // Drift first, whatever the build's outcome: a build of a source that has changed
        // since is stale whether it succeeded or failed — and a failure is often BECAUSE of
        // the edit (the build read the file while it was being rewritten, half written).
        // Reporting it as FAILED would leave the settled source unbuilt.
        let now = crate::file_access::read(&src).unwrap_or_default();
        if now != *snapshot {
            // Stale: the source changed while the build ran.  Requeue with the
            // current content — the cache makes an already-built version
            // instant, so this converges as soon as the source settles.
            eprintln!("loft-live: rebuild stale (source changed during build) — requeued");
            let driver = live_var("LOFT_LIVE_DRIVER").unwrap_or_default();
            *r = match spawn_build(&mut rc.seq, rc.ship, layout, &driver, &src) {
                Ok(b) => b,
                Err(msg) => {
                    eprintln!("loft-live: requeue failed — {msg}");
                    Rebuild::Failed
                }
            };
            return match &*r {
                Rebuild::Building { .. } => REBUILD_BUILDING,
                _ => REBUILD_FAILED,
            };
        }
        if !status.success() {
            let tail: String = crate::file_access::read_to_string(&*err_path)
                .unwrap_or_default()
                .lines()
                .rev()
                .take(5)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            eprintln!("loft-live: rebuild FAILED (old build keeps serving):\n{tail}");
            // The line that says why: the world check's refusal or the compiler's error
            // when there is one, else the last line the build printed.
            let why = tail
                .lines()
                .find(|l| l.contains("cannot take over the running world") || l.contains("error"))
                .or_else(|| tail.lines().last())
                .unwrap_or("the build failed")
                .trim()
                .to_string();
            rc.failure = why;
            *r = Rebuild::Failed;
            return REBUILD_FAILED;
        }
        // The artifact path rides the driver's ok line: "ok <src> <artifact>".
        let out = crate::file_access::read_to_string(&*out_path).unwrap_or_default();
        let artifact = out
            .lines()
            .find_map(|l| l.strip_prefix(&format!("ok {src} ")))
            .unwrap_or("")
            .trim()
            .to_string();
        if artifact.is_empty() || !crate::file_access::exists(&artifact) {
            eprintln!("loft-live: rebuild succeeded but no artifact on the ok line ({out:?})");
            *r = Rebuild::Failed;
            return REBUILD_FAILED;
        }
        eprintln!("loft-live: rebuild ready — {artifact}");
        *r = Rebuild::Ready { artifact };
        REBUILD_READY
    }
}

/// Control-channel shims (@PLN18 08-S7) — the kernel's debug handler drives
/// the rebuild machinery without reaching the private internals.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn n_rebuild_start_ctl(rc: &mut RebuildCx, layout: Option<&str>, ship: bool) -> bool {
    if ship {
        rebuild_start_tier(rc, layout, true)
    } else {
        rebuild_start(rc, layout)
    }
}

/// Is the ready build of the shipped tier (`rebuild fast`)?  Only that one is full speed.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rebuild_is_ship(rc: &mut RebuildCx, layout: Option<&str>) -> bool {
    rc.ship && rebuild_status(rc, layout) == REBUILD_READY
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rebuild_status_ctl(rc: &mut RebuildCx, layout: Option<&str>) -> i64 {
    rebuild_status(rc, layout)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rebuild_artifact_ctl(rc: &RebuildCx) -> String {
    rebuild_artifact(rc)
}

/// Why the last rebuild failed, in one line; "" when none has.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rebuild_failure_ctl(rc: &RebuildCx) -> String {
    rc.failure.clone()
}

/// The READY build's artifact path; "" unless `rebuild_status() == 2`.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_artifact(rc: &RebuildCx) -> String {
    match &rc.state {
        Rebuild::Ready { artifact } => artifact.clone(),
        _ => String::new(),
    }
}

/// The calling program's rebuild and the world layout it checks against.
#[cfg(not(target_arch = "wasm32"))]
#[expect(
    clippy::mut_from_ref,
    reason = "the stores sit in an `UnsafeCell`, the writer's calling convention"
)]
fn parts(cell: &UnsafeCell<Stores>) -> (&mut RebuildCx, Option<&str>) {
    // SAFETY: the cell is the calling program's stores, which no other reference reaches
    // while one of its natives runs.
    crate::engine_host::rebuild_parts(unsafe { &mut *cell.get() })
}

/// Typed twin: `rebuild_start() -> boolean`.
#[cfg(not(target_arch = "wasm32"))]
pub fn n_rebuild_start(cell: &UnsafeCell<Stores>) -> u8 {
    let (rc, layout) = parts(cell);
    u8::from(rebuild_start(rc, layout))
}

/// Typed twin: `rebuild_status() -> integer`.
#[cfg(not(target_arch = "wasm32"))]
pub fn n_rebuild_status(cell: &UnsafeCell<Stores>) -> i64 {
    let (rc, layout) = parts(cell);
    rebuild_status(rc, layout)
}

/// Typed twin: `kernel_rebuild_artifact() -> text` (owned String).
#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_rebuild_artifact(cell: &UnsafeCell<Stores>) -> String {
    rebuild_artifact(parts(cell).0)
}

/// Interp stack natives.  An interpreted serve host rebuilds too: the driver names the
/// program to the rebuild before it runs it ([`set_interp_host`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn n_rebuild_start_stack(stores: &mut Stores, stack: &mut DbRef) {
    let (rc, layout) = crate::engine_host::rebuild_parts(stores);
    let ok = rebuild_start(rc, layout);
    stores.put(stack, ok);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_rebuild_status_stack(stores: &mut Stores, stack: &mut DbRef) {
    let (rc, layout) = crate::engine_host::rebuild_parts(stores);
    let status = rebuild_status(rc, layout);
    stores.put(stack, status);
}

/// Destination-passing text return (the `is_text_dest_native` route).
#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_rebuild_artifact_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = stores.get::<DbRef>(stack);
    let v = rebuild_artifact(crate::engine_host::rebuild_parts(stores).0);
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}
