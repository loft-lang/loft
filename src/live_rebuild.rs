// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I78 — Live-reload dispatch: the background rebuild

//! The serve host's background rebuild: spawn the loft driver's `--check --native` as a
//! child, snapshot the source at request time, and requeue when it changed mid-build.  Only
//! process orchestration, so it sits in the runtime a compiled program links (@PLN191);
//! `live_dispatch` re-exports it.

use std::cell::{RefCell, UnsafeCell};

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
enum Rebuild {
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

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static REBUILD: RefCell<Rebuild> = const { RefCell::new(Rebuild::Idle) };
    static REBUILD_SEQ: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Spawn one background `--check --native` build of `src` via the driver.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_build(driver: &str, src: &str) -> Result<Rebuild, String> {
    // The error starts with the path: "cannot read <src>: <why>".
    let snapshot = crate::file_access::read(src).map_err(|e| format!("cannot read {e}"))?;
    let seq = REBUILD_SEQ.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
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
    if let Ok(libs) = std::env::var("LOFT_LIVE_LIBS") {
        for d in libs.split(':').filter(|s| !s.is_empty()) {
            cmd.push_arg("--lib").push_arg(d);
        }
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
fn rebuild_start() -> bool {
    let (Ok(driver), Ok(src)) = (
        std::env::var("LOFT_LIVE_DRIVER"),
        std::env::var("LOFT_LIVE_SRC"),
    ) else {
        eprintln!(
            "loft-live: rebuild needs LOFT_LIVE_DRIVER/LOFT_LIVE_SRC (run through the loft driver)"
        );
        return false;
    };
    REBUILD.with(|r| {
        let mut r = r.borrow_mut();
        if matches!(*r, Rebuild::Building { .. }) {
            return true; // idempotent: one build at a time
        }
        match spawn_build(&driver, &src) {
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
    })
}

/// Poll the rebuild: 0 idle, 1 building, 2 ready, 3 failed.  Completion
/// with a source that changed mid-build REQUEUES automatically (status
/// stays 1) — the artifact must correspond to the source as of the check.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_status() -> i64 {
    REBUILD.with(|r| {
        let mut r = r.borrow_mut();
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
        let src = std::env::var("LOFT_LIVE_SRC").unwrap_or_default();
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
            let driver = std::env::var("LOFT_LIVE_DRIVER").unwrap_or_default();
            *r = match spawn_build(&driver, &src) {
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
    })
}

/// Control-channel shims (@PLN18 08-S7) — the kernel's debug handler drives
/// the rebuild machinery without reaching the private internals.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn n_rebuild_start_ctl() -> bool {
    rebuild_start()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rebuild_status_ctl() -> i64 {
    rebuild_status()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rebuild_artifact_ctl() -> String {
    rebuild_artifact()
}

/// The READY build's artifact path; "" unless `rebuild_status() == 2`.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_artifact() -> String {
    REBUILD.with(|r| match &*r.borrow() {
        Rebuild::Ready { artifact } => artifact.clone(),
        _ => String::new(),
    })
}

/// Typed twin: `rebuild_start() -> boolean`.
#[cfg(not(target_arch = "wasm32"))]
pub fn n_rebuild_start(_cell: &UnsafeCell<Stores>) -> u8 {
    u8::from(rebuild_start())
}

/// Typed twin: `rebuild_status() -> integer`.
#[cfg(not(target_arch = "wasm32"))]
pub fn n_rebuild_status(_cell: &UnsafeCell<Stores>) -> i64 {
    rebuild_status()
}

/// Typed twin: `kernel_rebuild_artifact() -> text` (owned String).
#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_rebuild_artifact(_cell: &UnsafeCell<Stores>) -> String {
    rebuild_artifact()
}

/// Interp stack natives.  An interpreted serve host CAN rebuild too — the
/// machinery is env-driven, not tier-driven (the driver sets the live env
/// on native spawns only, so today these fire there as a warned no-op; an
/// interp host that exports the env gets the real thing for free).
#[cfg(not(target_arch = "wasm32"))]
pub fn n_rebuild_start_stack(stores: &mut Stores, stack: &mut DbRef) {
    stores.put(stack, rebuild_start());
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_rebuild_status_stack(stores: &mut Stores, stack: &mut DbRef) {
    stores.put(stack, rebuild_status());
}

/// Destination-passing text return (the `is_text_dest_native` route).
#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_rebuild_artifact_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = stores.get::<DbRef>(stack);
    let v = rebuild_artifact();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}
