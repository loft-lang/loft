// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-C-Nest` — `par` loops that run at the same time reach none of each other's state: two
//! loops in two `parallel` arms, and a loop nested in a `par` worker.  Each answers the value the
//! sequential loop would (`@FR-C-Det`) on both backends; and where the host's C compiler can build
//! with ThreadSanitizer, the C of the program runs under it with no data race reported — the
//! check that each worker's context is its own (`@FR-C-Own`), that shared stores are only read
//! (`@FR-C-Shared`), and that results cross back on the starting thread (`@FR-C-Adopt`).
//!
//! The values are computed by hand: `arm(b)` sums `2 (b + i)` for `i` in `0..200`, which is
//! `400 b + 39 800`; the nested loop sums `100 v + 2 450` for `v` in `10..18`, which is 30 400.
//!
//! @falsified-at: bd5f29be0 with a SABOTAGE — `loft_par_job_run` handing each worker the main
//! thread's context instead of its own (`@FR-C-Own` broken): the C value check fails and
//! ThreadSanitizer names the data race on the first run.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

const PROGRAM: &str = "struct Score {
  value: integer,
}

struct ScoreList {
  items: vector<Score>,
}

fn double_score(r: const Score) -> integer {
  r.value * 2
}

fn mk(n: integer, base: integer) -> ScoreList {
  l = ScoreList { items: [] };
  for i in 0..n {
    l.items += [Score { value: base + i }];
  }
  l
}

fn inner_sum(r: const Score) -> integer {
  l = mk(50, r.value);
  s = 0;
  for a in l.items par(b = double_score(a), 4) { s += b; }
  s
}

fn arm(base: integer) {
  l = mk(200, base);
  s = 0;
  for a in l.items par(b = double_score(a), 4) { s += b; }
  assert(s == 400 * base + 39800, \"arm {base}: {s}\");
}

fn main() {
  parallel {
    arm(1000);
    arm(2000);
  }
  outer = mk(8, 10);
  t = 0;
  for a in outer.items par(b = inner_sum(a), 4) { t += b; }
  assert(t == 30400, \"nested: {t}\");
  println(\"parpar ok {t}\");
}
";

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loft_par_contexts_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn program(dir: &Path) -> PathBuf {
    let file = dir.join("parpar.loft");
    fa::write(&file, PROGRAM).expect("write the program");
    file
}

fn run_on(backend: &str) {
    let dir = scratch(backend.trim_start_matches('-'));
    let file = program(&dir);
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg("--no-warnings")
        .arg(&file)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("loft runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("parpar ok 30400"),
        "{backend}: loops at once must answer the sequential values\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&dir);
}

#[test]
fn loops_at_once_answer_the_sequential_values_on_the_interpreter() {
    run_on("--interpret");
}

#[test]
fn loops_at_once_answer_the_sequential_values_on_the_c() {
    run_on("--native-c");
}

/// Whether `cc` builds and runs a ThreadSanitizer program here: a host whose compiler has no
/// ThreadSanitizer (MSVC, a cc without its runtime) answers no, and the check is skipped.
fn tsan_available(dir: &Path) -> bool {
    let src = dir.join("probe.c");
    let exe = dir.join("probe");
    fa::write(&src, "int main(void) { return 0; }\n").expect("write the probe");
    loft::platform::process::harness_command("cc")
        .args(["-fsanitize=thread", "-o"])
        .arg(&exe)
        .arg(&src)
        .output()
        .is_ok_and(|o| o.status.success())
        && loft::platform::process::harness_command(&exe)
            .output()
            .is_ok_and(|o| o.status.success())
}

#[test]
fn loops_at_once_race_on_nothing_under_thread_sanitizer() {
    let dir = scratch("tsan");
    if !tsan_available(&dir) {
        eprintln!("skipped: `cc` builds no ThreadSanitizer program on this host");
        return;
    }
    let file = program(&dir);
    let c = dir.join("parpar.c");
    let emitted = loft::platform::process::harness_command(loft_bin())
        .arg("--emit-c")
        .arg(&c)
        .arg(&file)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("loft --emit-c runs");
    assert!(
        emitted.status.success(),
        "--emit-c: {}",
        String::from_utf8_lossy(&emitted.stderr)
    );
    let exe = dir.join("parpar");
    let built = loft::platform::process::harness_command("cc")
        .args([
            "-std=c11",
            "-w",
            "-O1",
            "-g",
            "-fsanitize=thread",
            "-pthread",
        ])
        .args(["-DLOFT_CALL_CHAIN", "-DLOFT_POSIX_HOST"])
        .arg(&c)
        .arg("-o")
        .arg(&exe)
        .arg("-lm")
        .output()
        .expect("cc runs");
    assert!(
        built.status.success(),
        "cc: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    // A race shows only sometimes: five runs, each must answer and report none.
    for run in 0..5 {
        let out = loft::platform::process::harness_command(&exe)
            .env("TSAN_OPTIONS", "halt_on_error=1 exitcode=66")
            .output()
            .expect("the program runs");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success()
                && String::from_utf8_lossy(&out.stdout).contains("parpar ok 30400")
                && !stderr.contains("ThreadSanitizer"),
            "run {run}: a data race between loops at once, or a wrong value\n{stderr}"
        );
    }
    let _ = fa::remove_dir_all(&dir);
}

/// The spellings of thread-local state in the runtime's source: Rust's, and the C the runtime is
/// written in or emits (`LOFT_TLS` was the C runtime's macro for it; `LOFT_TLS_…` names are
/// other things).
const THREAD_LOCAL_SPELLINGS: [&str; 5] = [
    "thread_local!",
    "_Thread_local",
    "__thread ",
    "__declspec(thread)",
    "LOFT_TLS",
];

fn thread_local_sites(dir: &Path, out: &mut Vec<String>) {
    for entry in fa::read_dir(dir).expect("read a source directory") {
        let path = entry.os_spelling();
        if fa::is_dir(&path) {
            thread_local_sites(&path, out);
            continue;
        }
        if !["rs", "h", "c"].iter().any(|e| fa::has_extension(&path, e)) {
            continue;
        }
        let Ok(text) = fa::read_to_string(&path) else {
            continue;
        };
        for (nr, line) in text.lines().enumerate() {
            for spelling in THREAD_LOCAL_SPELLINGS {
                let mut from = 0;
                while let Some(at) = line[from..].find(spelling) {
                    let end = from + at + spelling.len();
                    let word_goes_on = line[end..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
                    if !(spelling == "LOFT_TLS" && word_goes_on) {
                        out.push(format!("{}:{}: {}", path.display(), nr + 1, line.trim()));
                    }
                    from = end;
                }
            }
        }
    }
}

/// `@FR-C-Own` — a thread's state is a value it is handed, never a thread-local: no Rust
/// `thread_local!`, and no C thread storage in the runtime or in the C it emits (@PLN226).  A
/// per-thread value belongs to the context it describes — `Stores`, `State`, `Data`, the C's
/// `loft_cx` — and anything threads share takes one of `@FR-C-Publish`'s forms
/// (THREADING.md § Each thread's own state).
///
/// @falsified-at: 9d5c47675 — 9 lines name `thread_local!` there (`call_frames.rs` 2,
/// `codegen_runtime.rs` 6, `live_dispatch.rs` 1: the rustc writer's runtime) -> none at fc31b0cb6.
#[test]
fn no_state_is_a_thread_local() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sites = Vec::new();
    thread_local_sites(&src, &mut sites);
    assert!(
        sites.is_empty(),
        "thread-local state in the runtime — make it a field of the context it describes \
         (THREADING.md § Each thread's own state):\n{}",
        sites.join("\n")
    );
}
