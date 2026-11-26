// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A loft program written as C (`--emit-c`) and built under AddressSanitizer and UBSan: the
//! build where the C runtime reading or writing past an allocation, or an arithmetic overflow
//! in it, ends the run with a report instead of passing unseen.  A host whose `cc` has no
//! sanitizer runtime answers `None` from [`available`], and the test that asked skips.

use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The flags every sanitized build takes.  `-w`: a unit's warnings are another test's subject.
const FLAGS: [&str; 9] = [
    "-std=c11",
    "-O1",
    "-g",
    "-pthread",
    "-fsanitize=address,undefined",
    "-fno-sanitize-recover=all",
    "-fno-omit-frame-pointer",
    "-DLOFT_CALL_CHAIN",
    "-w",
];

/// The options a sanitized run takes: no leak check (macOS's runtime has none, and a leak is
/// not what these tests ask), and an allocation too large to make answers null, so the
/// runtime's own "out of memory" is what a too-large request shows.
pub const ASAN_OPTIONS: &str = "detect_leaks=0:allocator_may_return_null=1";

/// Does `cc` build and run a program under AddressSanitizer and UBSan here?
#[must_use]
pub fn available(dir: &Path) -> bool {
    let src = dir.join("sanitizer_probe.c");
    let exe = dir.join("sanitizer_probe");
    if fa::write(&src, "int main(void) { return 0; }\n").is_err() {
        return false;
    }
    loft::platform::process::harness_command("cc")
        .args(FLAGS)
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .is_ok_and(|o| o.status.success())
        && loft::platform::process::harness_command(&exe)
            .env("ASAN_OPTIONS", ASAN_OPTIONS)
            .output()
            .is_ok_and(|o| o.status.success())
}

/// Write `prog` as C into `dir` (`<stem>.c`): the program and the whole runtime, one unit.
#[must_use]
pub fn emit(dir: &Path, prog: &Path) -> PathBuf {
    let stem = fa::file_stem(prog).unwrap_or_else(|| "p".into());
    let c = dir.join(format!("{stem}.c"));
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--emit-c")
        .arg(&c)
        .arg(prog)
        .output()
        .expect("run loft --emit-c");
    assert!(
        out.status.success(),
        "{stem}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    c
}

/// Build the C file `c` under the sanitizers, linked as the driver links the unit it includes
/// (`unit`, the emitted file — `c` itself, or a harness that `#include`s it).
#[must_use]
pub fn build(c: &Path, unit: &Path) -> PathBuf {
    let exe = fa::with_extension(c, "");
    let inc = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/c");
    let links: Vec<String> = fa::read_to_string(unit)
        .expect("read the C")
        .lines()
        .filter_map(|l| l.strip_prefix("/* link: ")?.strip_suffix(" */"))
        .map(str::to_string)
        .chain(
            loft::platform::c_system_link_libs()
                .iter()
                .map(|l| (*l).to_string()),
        )
        .collect();
    let out = loft::platform::process::harness_command("cc")
        .args(FLAGS)
        .arg(format!("-I{}", inc.display()))
        .arg(c)
        .args(&links)
        .arg("-o")
        .arg(&exe)
        .args(["-lm", "-ldl"])
        .output()
        .expect("run cc");
    assert!(
        out.status.success(),
        "{}: {}",
        c.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    exe
}

/// What a sanitized run did: its exit code (`None`: killed, or still running at the deadline),
/// whether it was stopped at the deadline, and its two streams.
pub struct Run {
    pub code: Option<i32>,
    pub timed_out: bool,
    pub out: String,
    pub err: String,
}

impl Run {
    /// Did a sanitizer report an error?  (A warning — an allocation too large to make,
    /// answered null — is not one.)
    #[must_use]
    pub fn reported(&self) -> bool {
        self.err.contains("ERROR: AddressSanitizer") || self.err.contains("runtime error:")
    }
}

/// Run `exe` with `args` in `cwd` under [`ASAN_OPTIONS`] and `env`, stopped after `limit`.
#[must_use]
pub fn run(exe: &Path, args: &[&str], cwd: &Path, env: &[(&str, &str)], limit: Duration) -> Run {
    let mut cmd = loft::platform::process::harness_command(exe);
    cmd.args(args)
        .current_dir(cwd)
        .env("ASAN_OPTIONS", ASAN_OPTIONS)
        .env_remove("LOFT_SOURCE_DIR")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("run the program");
    let deadline = Instant::now() + limit;
    let mut timed_out = false;
    while child.try_wait().ok().flatten().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            timed_out = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().expect("the program's output");
    Run {
        code: if timed_out { None } else { out.status.code() },
        timed_out,
        out: String::from_utf8_lossy(&out.stdout).into_owned(),
        err: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}
