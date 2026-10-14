// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Shared helpers for integration test binaries.
//!
//! Each item is `#[allow(dead_code)]` because this module is pulled
//! into multiple test binaries via `mod common;`, and not every
//! binary uses every helper.  Without the allow, binaries that don't
//! consume a given helper produce a warning that turns into a CI
//! failure under `-D warnings`.

#[allow(dead_code)]
pub mod cross_mode;

#[allow(dead_code)]
pub mod timing;

/// Hold this while a corpus program RUNS: a cross-process advisory lock keyed by the program's
/// absolute path (loft#1724).
///
/// A program runs from the repository root, so the file names it writes are shared by every
/// harness that runs it — `wrap` (the interpreter) and `native` (the compiled binary) run the
/// same `tests/docs` and `tests/scripts` files in separate nextest processes at once, and
/// `13-file.loft`'s `test.bin` written by one was read back by the other.  An in-process mutex
/// cannot see the other binary; this lock can, and serialises only runs of the SAME file.  The
/// key is the absolute path, so two checkouts never wait on each other.  Best-effort: a lock
/// that cannot be opened is `None` and the run proceeds unserialised, as before.  Take it once
/// per run — a second `lock` on a fresh handle in the same process blocks on the first.
#[allow(dead_code)]
#[must_use]
pub fn source_run_lock(source: &std::path::Path) -> Option<std::fs::File> {
    use std::hash::{Hash, Hasher};
    let abs = std::path::absolute(source).unwrap_or_else(|_| source.to_path_buf());
    let mut h = std::collections::hash_map::DefaultHasher::new();
    abs.hash(&mut h);
    let lock = fa::open_with(
        std::env::temp_dir().join(format!("loft-run-{:016x}.lock", h.finish())),
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false),
    )
    .ok()?;
    lock.lock().ok()?;
    Some(lock)
}

/// The `file://` URL for a local path, in the one shape this repo's registry fixtures and
/// `registry_index::http_get_bytes` both handle.
///
/// `http_get_bytes` strips the `file://` prefix and hands the remainder straight to
/// `std::fs::read`, so the URL carries a NATIVE path — not an RFC `file:///D:/…` URI, whose
/// leading slash that reader would keep.  What the naive `format!("file://{}", p.display())`
/// gets wrong is Windows separators: a fixture index is JSON, and `\U` / `\r` / `\t` are
/// not valid JSON string escapes, so loft's own parser correctly refused the index the
/// fixture had just written (`invalid escape \U`, the nightly's Windows leg).  Windows
/// accepts `/` in a path, so rendering separators as `/` is JSON-safe and unchanged on unix.
#[allow(dead_code)]
#[must_use]
pub fn file_url(p: &std::path::Path) -> String {
    format!("file://{}", p.display().to_string().replace('\\', "/"))
}

/// How much to stretch a test's wall-clock deadline, because the machine is shared.
///
/// A deadline is an UPPER bound: a fast run returns early and pays nothing for a
/// generous budget, while a tight one turns ordinary contention into a failure.  So the
/// question is not "how long should this take" but "am I sharing the box".
///
/// `CI` alone misses the case that actually bites: a LOCAL full-suite run, where dozens
/// of tests share the CPU — measured at 61.6 s against a 60 s budget for a browser test
/// that takes 25 s alone.  `NEXTEST` covers it, since the harness is exactly what runs
/// tests in parallel.  A hand-run test binary keeps the tight budget, so iterating on one
/// test still fails fast.
#[allow(dead_code)]
#[must_use]
pub fn deadline_scale() -> u64 {
    let shared = std::env::var_os("CI").is_some() || std::env::var_os("NEXTEST").is_some();
    if shared { 3 } else { 1 }
}

/// Run `f` on a thread with the `loft` CLI's own stack: 8 MiB, or `RUST_MIN_STACK` when a
/// sanitizer job raises it.  A test thread has 2 MiB on Linux and less on Windows, and the
/// front end spends ~22 KiB of stack per nesting level of an expression, so a corpus program
/// the CLI compiles (the 100-term sum in `a-frame-never-pushes-past-the-room-its-entry-made`)
/// overflowed a test that walked the corpus on its own thread.  A panic re-raises here, so the
/// test fails exactly as it would have.
#[allow(dead_code)]
pub fn on_cli_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    let stack = std::env::var("RUST_MIN_STACK")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map_or(8 << 20, |v| v.max(8 << 20));
    std::thread::scope(|sc| {
        let run = std::thread::Builder::new()
            .stack_size(stack)
            .spawn_scoped(sc, f)
            .expect("spawn the CLI-stack thread");
        match run.join() {
            Ok(v) => v,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    })
}

/// A server-test port, isolated PER CHECKOUT.  The engine-host / wasm-relay tests bind FIXED
/// ports; two suites run at once — e.g. two agents in sibling checkouts (`loft` and `loft2`) —
/// collide on them and flake, and the red reads as a browser or kernel defect rather than as a
/// port that belongs to someone else (loft#1193).
///
/// The offset is derived from THIS checkout's path, so every way of running the suite gets it:
/// `make ci`, a bare `cargo test`, and `find_problems.sh` alike.  It used to be computed by
/// `find_problems.sh` and exported, which left `make ci` — the gate two agents most often run
/// at the same time — on the base ports with nothing separating the two runs.
///
/// `LOFT_TEST_PORT_OFFSET` still overrides it, for pinning a range by hand.
/// A port this test can actually BIND — measured, not assumed.
///
/// [`test_port`] computes the port a suite is SUPPOSED to use; this one guarantees the
/// test can have it, and is what a test that spawns a server should call.  When the
/// canonical port is taken there are two situations, and they deserve opposite answers:
///
///  1. **Our own leaked artifact** — reap it and keep the canonical port.  The cascade is
///     real and was observed: nextest SIGKILLs a timed-out test, so the process-group
///     `Guard::drop` never runs, so the server survives on its fixed port, so the NEXT run
///     cannot bind and times out too, leaking another orphan.  Four were found live on one
///     box, two holding ports the engine-host suite needs, one pair 14 hours old.  Our own
///     mess is ours to clear, and keeping the canonical port keeps failures readable.
///
///  2. **Anyone else's process** — PIVOT to a free port and leave it alone.  Killing is
///     the wrong default: a sibling checkout's suite, or an unrelated program, is not this
///     test's to destroy, and a test that clears its path by killing whatever is in the way
///     just moves the flake onto someone else.  The port offsets exist to avoid collisions;
///     pivoting is how a test copes when one happens anyway.
///
/// Moving is cheap because the port is a pure parameter — the fixture is generated with it
/// and the child is told it — so nothing else has to agree on a fixed number.
#[allow(dead_code)]
pub fn bind_port(base: u16) -> u16 {
    // IDEMPOTENT per process, which nextest makes per TEST.  [`test_port`] is a pure
    // function and callers rely on that without saying so: `engine_host_reload` asks for
    // its port once in the test and again inside the fixture writer, and before this cache
    // the two calls pivoted independently — the server bound one port while the client
    // dialled another, and the test failed inside `ws_recv` with nothing about ports in the
    // message.  Resolving a base once and remembering it keeps the drop-in substitution for
    // `test_port` honest: same base, same answer, however many times it is asked.
    static RESOLVED: std::sync::Mutex<Option<Vec<(u16, u16)>>> = std::sync::Mutex::new(None);
    if let Ok(g) = RESOLVED.lock()
        && let Some(v) = g.as_ref()
        && let Some(&(_, p)) = v.iter().find(|(b, _)| *b == base)
    {
        return p;
    }
    let port = resolve_bindable_port(base);
    if let Ok(mut g) = RESOLVED.lock() {
        g.get_or_insert_with(Vec::new).push((base, port));
    }
    port
}

fn resolve_bindable_port(base: u16) -> u16 {
    let canonical = test_port(base);
    // A canonical port inside the kernel's ephemeral range is unusable however free it
    // looks: the kernel draws outgoing-connection ports from there, so one can be taken
    // between this check and the child's bind.  Pivot below the floor rather than trust a
    // free-right-now answer.  `find_problems.sh` bounds its offset to keep ports out of that
    // range, and this is the belt to that braces — a hand-set `LOFT_TEST_PORT_OFFSET` gets
    // the same protection.
    if canonical >= ephemeral_floor() {
        let safe = pivot_port(base).unwrap_or(canonical);
        eprintln!("[port] canonical {canonical} is inside the ephemeral range — using {safe}");
        return safe;
    }
    if port_is_free(canonical) {
        return canonical;
    }
    if reap_our_leaked_holders(canonical) {
        return canonical;
    }
    let pivot = pivot_port(pivot_anchor(canonical)).unwrap_or(canonical);
    eprintln!("[port] {canonical} unavailable and not ours to kill — pivoting to {pivot}");
    pivot
}

/// Where a busy canonical port's pivot ladder starts: the canonical port itself when it sits
/// low enough for the whole ladder to stay below the ephemeral floor, and otherwise a point in
/// the 2000 ports under that band chosen BY the canonical port.
///
/// The anchor has to keep distinct canonical ports distinct, because two tests of one run can
/// pivot at the same moment, and each takes the first free rung it sees before either child
/// binds.  Clamping every high canonical port to one value (`canonical.min(floor - 8000)`)
/// sent `engine_host_kernel`'s `s1` (30094) and its debugger test (31312) both to 24869 while
/// a leaked server held their own ports: the file failed in parallel and passed serially.
/// Canonical ports within one run differ by less than 2000, so their residues differ too.
#[allow(dead_code)]
fn pivot_anchor(canonical: u16) -> u16 {
    let band = ephemeral_floor().saturating_sub(8000);
    if canonical < band {
        canonical
    } else {
        band.saturating_sub(2000) + canonical % 2000
    }
}

/// Kill this checkout's own leaked holders of `port`, and answer whether the port came
/// free.  `false` when there were none, when a holder was not ours to kill, or when the
/// port stayed busy.
///
/// UNIX ONLY, and the whole mechanism is: it finds holders with `lsof`, proves ownership
/// through `/proc/<pid>/exe`, and kills with `kill(2)`.  Windows has none of the three, so
/// the port there is simply pivoted around instead — which is what the caller already does
/// for a port that is not ours.
// approved exemption: `lsof`, `/proc/<pid>/exe` and `kill(2)` have no Windows equivalent; Windows substitute: the caller pivots to another port
#[cfg(unix)]
#[allow(dead_code)]
fn reap_our_leaked_holders(port: u16) -> bool {
    let Some(pids) = holders_owned_by_this_checkout(port) else {
        return false;
    };
    eprintln!("[port] {port} held by our own leaked pid(s) {pids:?} — reaping");
    for pid in pids {
        // SAFETY: an ordinary kill(2) on a pid we just proved runs this checkout's
        // own build artifact.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    // SIGKILL is asynchronous and SO_REUSEPORT lets a dying listener still accept, so
    // poll for the port to be genuinely bindable rather than racing onto a stale world.
    for _ in 0..40 {
        if port_is_free(port) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

/// Windows has no `lsof`, no `/proc` and no `kill(2)`, so there is nothing to reap — the
/// caller pivots to another port, exactly as it does for a holder that is not ours.
// approved exemption: `lsof`, `/proc/<pid>/exe` and `kill(2)` have no Windows equivalent; Windows substitute: the caller pivots to another port
#[cfg(not(unix))]
#[allow(dead_code)]
fn reap_our_leaked_holders(_port: u16) -> bool {
    false
}

/// Has this process been reparented to init — i.e. did whoever spawned it die?
///
/// `/proc/<pid>/stat` field 4 is the ppid, but fields 1..2 are the pid and the comm, and a
/// comm may itself contain spaces and parentheses — so parse AFTER the last `)`, which is
/// the documented way to read this file.
#[allow(dead_code)]
fn is_orphan(pid: i32) -> bool {
    let Ok(stat) = fa::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else {
        return false;
    };
    // rest = " S <ppid> ..." — state, then parent pid.
    let Some(ppid) = rest
        .split_whitespace()
        .nth(1)
        .and_then(|p| p.parse::<i32>().ok())
    else {
        return false;
    };
    if ppid == 1 {
        return true;
    }
    // An orphan is not always init's: a session under `systemd --user` reparents to
    // that subreaper (pid ≠ 1), and every leaked engine host on such a box read as
    // "still parented" here, so the reaper never fired and each run pivoted past the
    // previous run's orphan and left one more.  The question is whether the PARENT is
    // one of this checkout's processes — a live sibling test's server has a test binary
    // under this tree as its parent; a leaked one has init or a subreaper.  Unreadable
    // is not an orphan: the conservative direction is to decline the kill.
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    match fa::read_link(format!("/proc/{ppid}/exe")) {
        Ok(parent_exe) => !parent_exe.starts_with(&root),
        Err(_) => false,
    }
}

/// Can this process bind the port right now?  Binding and dropping is the only honest
/// test: `lsof` answers who holds a LISTENING socket, not whether a bind would succeed.
#[allow(dead_code)]
fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind(("0.0.0.0", port)).is_ok()
}

/// A free port to move to, chosen BELOW the kernel's ephemeral range.
///
/// The obvious implementation — bind `:0` and take what the OS assigns — is wrong here,
/// and measurably so: it hands back a port from `ip_local_port_range` (32768–60999 on this
/// box), which is the very range the kernel draws from for OUTGOING connections. Between
/// releasing it and the child binding it, any outbound socket on the machine can take it,
/// including one the test itself opens. Doing that turned a passing `engine_host_reload`
/// into a reliable failure inside `ws_recv`.
///
/// So scan a deterministic ladder below the ephemeral floor instead. The stride is coprime
/// with the 100-spacing the suites' base ports use, so a pivot does not land on another
/// test's canonical port.
#[allow(dead_code)]
fn pivot_port(anchor: u16) -> Option<u16> {
    let floor = ephemeral_floor();
    (1..=64u16).find_map(|k| {
        let cand = anchor.checked_add(k.checked_mul(101)?)?;
        (cand < floor && port_is_free(cand)).then_some(cand)
    })
}

/// The lowest port the kernel may hand out for an outgoing connection.  Anything at or
/// above this can be taken from under a test between the check and the bind.
#[allow(dead_code)]
fn ephemeral_floor() -> u16 {
    fa::read_to_string("/proc/sys/net/ipv4/ip_local_port_range")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse().ok())
        .unwrap_or(32768)
}

/// The pids holding `port` that are genuinely LEAKED by this checkout — the only ones
/// safe to kill.
///
/// Two conditions, and the second was learned the hard way.  The holder's executable must
/// live under this checkout (never kill a foreign process), AND it must be an ORPHAN —
/// reparented to init because the test that spawned it is gone.
///
/// Ownership alone is not enough: two tests in `engine_host_reload` share one `PORT_BASE`,
/// so the second to start found the port held by the FIRST test's live server, correctly
/// judged it "ours", and killed it.  That turned a flake into a reliable failure — worse
/// than the problem being fixed.  A live sibling's server has a living parent; a leaked one
/// does not, and that is the difference between cleaning up after ourselves and shooting a
/// test that is still running.
#[allow(dead_code)]
fn holders_owned_by_this_checkout(port: u16) -> Option<Vec<i32>> {
    let out = loft::platform::process::harness_command("lsof")
        .arg("-ti")
        .arg(format!("tcp:{port}"))
        .output()
        .ok()?;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut ours = Vec::new();
    for pid in String::from_utf8_lossy(&out.stdout).split_whitespace() {
        let pid: i32 = pid.parse().ok()?;
        let exe = fa::read_link(format!("/proc/{pid}/exe")).ok()?;
        if !exe.starts_with(&root) {
            return None; // a foreign holder — the whole port is off limits
        }
        if !is_orphan(pid) {
            return None; // ours, but still parented: a LIVE sibling test's server
        }
        ours.push(pid);
    }
    (!ours.is_empty()).then_some(ours)
}

#[allow(dead_code)]
pub fn test_port(base: u16) -> u16 {
    let offset = std::env::var("LOFT_TEST_PORT_OFFSET")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or_else(checkout_port_offset);
    base.saturating_add(offset)
}

/// This checkout's port band: `(cksum(path) % 6 + 1) * 2000`, derived from the checkout's own
/// path.
///
/// **2000 is wider than the 1236 the suites' base ports span** (18087–19322), so two checkouts
/// on different bands can never overlap.
///
/// **The multiplier stops at six because of the kernel's ephemeral range**
/// (`ip_local_port_range`, 32768 on Linux): the top base port is 19322, so 19322 + 12000 =
/// 31322 stays below it.  A port at or above that floor can be taken from under a test between
/// the availability check and the server's bind by ANY outbound socket on the box — cargo,
/// rustc, a browser a test launches — and that was the whole of the long-standing
/// wasm_debug_relay / engine_host flake back when the multiplier allowed offsets up to 32000.
///
/// **Six bands is a spread, not a guarantee**, and saying so is the point: two checkouts can
/// still hash to one band, and what covers that is [`bind_port`]'s pivot — it declines to kill
/// a holder that is not ours and moves. The band is what makes reaching that path rare; it is
/// not what makes a collision impossible.
///
/// The path is read at COMPILE time (`CARGO_MANIFEST_DIR`), which is what makes the band a
/// property of the checkout rather than of whoever launched the run.
#[allow(dead_code)]
fn checkout_port_offset() -> u16 {
    port_band_of(env!("CARGO_MANIFEST_DIR"))
}

/// [`checkout_port_offset`] for an arbitrary path — the testable half, so the formula can be
/// checked without moving this checkout.
#[allow(dead_code)]
#[must_use]
pub fn port_band_of(path: &str) -> u16 {
    u16::try_from(u64::from(posix_cksum(path.as_bytes())) % 6 + 1).unwrap_or(1) * 2000
}

/// POSIX `cksum` — CRC-32 over the bytes, then over the byte length, complemented.
///
/// This is the checksum `find_problems.sh` piped the checkout path through for as long as the
/// per-checkout band has existed, and reproducing it exactly is deliberate: the bands the two
/// tools hand out are then the SAME bands, so moving the formula here does not silently
/// re-shuffle which checkout sits where.  It is also the one property of this function that can
/// be asserted against something outside the repo — `cksum` itself.
#[allow(dead_code)]
fn posix_cksum(bytes: &[u8]) -> u32 {
    fn feed(crc: u32, b: u8) -> u32 {
        let mut c = crc ^ (u32::from(b) << 24);
        for _ in 0..8 {
            c = if c & 0x8000_0000 != 0 {
                (c << 1) ^ 0x04c1_1db7
            } else {
                c << 1
            };
        }
        c
    }
    let mut crc = 0_u32;
    for b in bytes {
        crc = feed(crc, *b);
    }
    let mut len = bytes.len() as u64;
    while len != 0 {
        crc = feed(crc, (len & 0xff) as u8);
        len >>= 8;
    }
    !crc
}

use loft::data::Data;
use loft::database::Stores;
use loft::file_access as fa;
use loft::parser::Parser;
use std::path::PathBuf;
use std::sync::OnceLock;

/// On Windows MSVC, the build-script output dirs holding native import libraries
/// (e.g. `windows.0.48.5.lib` from `windows-sys`) must be passed to a hand-driven
/// `rustc` as `-L` paths — cargo adds them via `cargo:rustc-link-search` but a
/// test that links a cdylib by hand does not, so the link fails
/// `LNK1181: cannot open input file …`.  Mirrors `native_lib::native_lib_search_dirs`
/// and the `--native` test runner.  Empty (a no-op) off Windows.
///
/// `rlib` is `target/<profile>/libloft.rlib` or `target/<profile>/deps/libloft-*.rlib`.
#[allow(dead_code)]
pub fn native_lib_search_dirs(rlib: &std::path::Path) -> Vec<PathBuf> {
    if loft::platform::host_lib_os() != loft::platform::LibOs::Windows {
        return Vec::new();
    }
    // Walk up to the profile dir (release/ or debug/), then scan `build/<crate>-<hash>/`.
    let Some(profile_dir) = rlib.parent().and_then(|p| {
        if fa::file_name(p).is_some_and(|n| n == "deps") {
            p.parent()
        } else {
            Some(p)
        }
    }) else {
        return Vec::new();
    };
    let Ok(entries) = fa::read_dir(profile_dir.join("build")) else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    for entry in entries {
        let build_entry = entry.os_spelling();
        // `out/` and its immediate subdirs (libs generated into OUT_DIR).
        let out = build_entry.join("out");
        if fa::is_dir(&out) {
            dirs.push(out.clone());
            if let Ok(subs) = fa::read_dir(&out) {
                dirs.extend(
                    subs.into_iter()
                        .map(|e| e.os_spelling())
                        .filter(|p| fa::is_dir(p)),
                );
            }
        }
        // `cargo:rustc-link-search` directives cached in `build/<crate>-<hash>/output`
        // (e.g. `windows_x86_64_msvc` ships its `.lib` inside the registry package).
        if let Ok(content) = fa::read_to_string(build_entry.join("output")) {
            for line in content.lines() {
                if let Some(p) = line
                    .strip_prefix("cargo:rustc-link-search=native=")
                    .or_else(|| line.strip_prefix("cargo:rustc-link-search="))
                {
                    let p = PathBuf::from(p);
                    if fa::is_dir(&p) && !dirs.contains(&p) {
                        dirs.push(p);
                    }
                }
            }
        }
    }
    dirs
}

/// Count the warnings **loft itself** raised about `script_name` (a `.loft` file name).
///
/// Use this instead of counting every `warning:` line whenever a test spawns the loft binary:
/// a `--native` run relays rustc's whole stderr verbatim (`src/main.rs`, "Relay rustc's own
/// output"), and rustc opens its diagnostics with the same `warning:` header, so a bare count
/// also counts the toolchain's.  That difference is invisible on Linux — where the generated
/// crate compiles clean — and shows up only on another host: on `windows-latest` an MSVC
/// linker warning plus rustc's `warning: N warnings emitted` summary added two phantom
/// warnings and failed a test that passes everywhere else.
///
/// Attribution keys on the location every loft diagnostic carries — `  --> <script>:<line>:<col>`
/// on the line below the header (pretty, the default) or ` at <script>:<line>:<col>` on the
/// header itself (compact, `LOFT_ERRORS=compact`).  rustc's diagnostics point at the generated
/// `loft_native_<pid>.rs`, or carry no location at all, so neither is ever counted.
#[allow(dead_code)]
pub fn loft_warnings(stderr: &str, script_name: &str) -> usize {
    let lines: Vec<&str> = stderr.lines().collect();
    let mut count = 0;
    for (i, line) in lines.iter().enumerate() {
        // Compact: `Warning[code]: <message> at <file>:<line>:<col>` — one line, self-locating.
        if line.starts_with("Warning") && line.contains(script_name) {
            count += 1;
            continue;
        }
        // Pretty: `warning[code]: <message>` followed by the `-->` location line.
        let header = line.starts_with("warning:") || line.starts_with("warning[");
        let points_at_script = lines
            .get(i + 1)
            .is_some_and(|loc| loc.trim_start().starts_with("-->") && loc.contains(script_name));
        if header && points_at_script {
            count += 1;
        }
    }
    count
}

/// Record environmental skips — tests that PASSED-by-skipping for a
/// toolchain/OS reason rather than a code reason — to a side-channel ledger, so
/// they survive nextest's suppression of successful output.
///
/// Reach for this from any test that self-skips on a missing toolchain.  A skip
/// and a pass are indistinguishable in a summary, so without the ledger a green
/// run hides reduced coverage: the regression of whatever the test guards looks
/// exactly like a clean run.  The CI step `Surface environmental test skips`
/// drains the ledger into annotations and a job summary, which is what turns
/// "more tests skip than yesterday" into something visible.
///
/// No-op unless `LOFT_SKIP_LEDGER` (a directory) is set, so local runs are
/// unaffected.  Each call gets its own file — pid-named for the cross-process
/// case (nextest runs one process per test) and counter-suffixed for the
/// same-process one (`cargo test` runs a binary's tests as threads), so a
/// second caller can never truncate the first one's record.
#[allow(dead_code)]
pub fn record_env_skips(suite: &str, reason: &str, skips: &[(String, String)]) {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let Ok(dir) = std::env::var("LOFT_SKIP_LEDGER") else {
        return;
    };
    if fa::create_dir_all(&dir).is_err() {
        return;
    }
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::path::Path::new(&dir).join(format!("{suite}-{}-{seq}.tsv", std::process::id()));
    let body: String = skips
        .iter()
        .map(|(entry, detail)| {
            let clean = |s: &str| s.replace(['\t', '\n'], " ");
            format!("{suite}\t{reason}\t{}\t{}\n", clean(entry), clean(detail))
        })
        .collect();
    let _ = fa::write(path, body);
}

#[allow(dead_code)]
static DEFAULT_PARSED: OnceLock<(Data, Stores)> = OnceLock::new();

#[allow(dead_code)]
static DEFAULT_FROM_SOURCE: OnceLock<(Data, Stores)> = OnceLock::new();

/// The standard library a test starts from, once per process: warm-loaded from the startup
/// cache's stdlib bundle when one is current, else parsed and saved for the next process.
///
/// Under nextest every test is its own process, so a parse here is paid per TEST (~50 ms,
/// most of what a typical `code!` test costs).  The bundle's key covers the stdlib's content,
/// this binary's build and the semantics switches (`cache::stdlib_cache_key`), so an edited
/// `default/` or another build lands at a fresh key and is parsed; `save_bundle` publishes by
/// rename, so concurrent tests never read a partial file.  `LOFT_NO_CACHE` turns it off.
#[allow(dead_code)]
pub fn cached_default() -> (Data, Stores) {
    let (data, db) = DEFAULT_PARSED.get_or_init(|| {
        let mut p = Parser::new();
        if loft::startup_cache::warm_load_stdlib(&mut p, "default") {
            return (p.data, p.database);
        }
        p.parse_dir("default", true, false).unwrap();
        loft::startup_cache::save_stdlib_cache(&p, "default");
        (p.data, p.database)
    });
    (data.clone(), db.clone())
}

/// The standard library PARSED from source, never read back from a bundle — for a test of
/// the bundle codec itself, whose oracle must not be the codec's own output.
#[allow(dead_code)]
pub fn parsed_default() -> (Data, Stores) {
    let (data, db) = DEFAULT_FROM_SOURCE.get_or_init(|| {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        (p.data, p.database)
    });
    (data.clone(), db.clone())
}

/// The one reader for a `// @EXPECT_…` annotation, so the interpreter and native runners
/// cannot disagree about what a file DECLARES.
///
/// An annotation is a comment line whose text begins with the tag; anything else — a
/// sentence in the file header that happens to name the tag — declares nothing.  The
/// distinction is load-bearing because the native runner skips a whole file on the
/// strength of it: reading the tag with a plain `contains` dropped five scripts from that
/// suite for a comment recording that the file had STOPPED being an expected-error case,
/// `93-vector-advanced.loft`'s forty-nine assertions among them.
///
/// Returns the text after the tag, so a caller that wants the pattern and a caller that
/// only wants "is one present" read the same rule.
#[allow(dead_code)]
pub fn expect_tag<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let comment = line.trim().strip_prefix("//")?;
    comment.trim().strip_prefix(tag)
}

/// Does this source declare an expected parse/scope ERROR?  Such a file never reaches
/// execution — `wrap` stops at "errors consumed" and the native runner skips it — so
/// every runtime assertion in it is inert.
#[allow(dead_code)]
pub fn declares_expect_error(source: &str) -> bool {
    source
        .lines()
        .any(|l| expect_tag(l, "@EXPECT_ERROR:").is_some())
}

/// Does this source declare an expected FAILURE (a compile panic or a failing assert)?
#[allow(dead_code)]
pub fn declares_expect_fail(source: &str) -> bool {
    source
        .lines()
        .any(|l| expect_tag(l, "@EXPECT_FAIL").is_some())
}

/// Split a source's `@EXPECT_FAIL` declarations into the two kinds the annotation has:
/// the function names a FN-LEVEL tag names, and whether a FILE-LEVEL tag is present.
///
/// The tag is defined by POSITION, not by words on the line — a file-level one sits in the
/// header above the first declaration, a fn-level one is the comment block immediately
/// above its `fn`.  That is the rule the documentation states, so it is the rule both
/// runners read; a second parser keyed on something else is how the two drifted.  The
/// native runner hand-rolled `split_whitespace().skip_while(|w| *w != "@EXPECT_FAIL")`,
/// which cannot see the documented `// @EXPECT_FAIL: <reason>` form at all — the token
/// carries the colon — so its skip-set came back empty for every file written the
/// documented way, and the file-level drop above it silently cost each of those files its
/// whole native coverage (loft#1311).
///
/// A trailing `pending` tag — a comment declaring the tag with no declaration under it —
/// counts as file-level, because it governs nothing narrower.
#[allow(dead_code)]
pub fn expect_fail_fns(source: &str) -> (std::collections::HashSet<String>, bool) {
    let mut fns = std::collections::HashSet::new();
    let mut file_level = false;
    let mut pending = false;
    let mut in_header = true;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("fn ") {
            in_header = false;
            if pending
                && let Some(name) = trimmed
                    .strip_prefix("fn ")
                    .and_then(|s| s.split(&['(', ' ', '{'][..]).next())
            {
                fns.insert(name.to_string());
            }
            pending = false;
            continue;
        }
        if trimmed.starts_with("struct ") || trimmed.starts_with("enum ") {
            in_header = false;
            pending = false;
            continue;
        }
        if trimmed.starts_with("//") {
            if expect_tag(trimmed, "@EXPECT_FAIL").is_some() {
                if in_header {
                    file_level = true;
                } else {
                    pending = true;
                }
            }
        } else {
            pending = false;
        }
    }
    if pending {
        file_level = true;
    }
    (fns, file_level)
}

/// One `loft` sample of `doc/learn-loft.md`: the line its fence opens on, its code, and the
/// output the page shows for it — the plain fence after the next `Output:` line, when one
/// comes before the next `loft` fence.
#[allow(dead_code)]
pub struct LearnSample {
    pub line: usize,
    pub code: String,
    pub output: Option<String>,
}

/// Every `loft` sample of the tutorial, with the output the page claims for it (loft#1809).
///
/// The tutorial is the page a new user reads first, so each sample is run on both backends
/// (`wrap::learn_loft_samples`, `native::native_learn_loft_samples`) through
/// [`check_learn_sample`].
#[allow(dead_code)]
pub fn learn_loft_samples() -> Vec<LearnSample> {
    let page = fa::read_to_string("doc/learn-loft.md").expect("doc/learn-loft.md");
    let lines: Vec<&str> = page.lines().collect();
    let fence_end = |from: usize| (from..lines.len()).find(|&j| lines[j].trim() == "```");
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() != "```loft" {
            i += 1;
            continue;
        }
        let end = fence_end(i + 1).expect("an unclosed loft fence in learn-loft.md");
        let code = lines[i + 1..end].join("\n") + "\n";
        let mut output = None;
        let mut j = end + 1;
        while j < lines.len() && lines[j].trim() != "```loft" {
            if lines[j].trim() == "Output:" {
                let open = (j + 1..lines.len()).find(|&k| lines[k].trim().starts_with("```"));
                if let Some(open) = open {
                    let close = fence_end(open + 1).expect("an unclosed output fence");
                    output = Some(lines[open + 1..close].join("\n"));
                }
                break;
            }
            j += 1;
        }
        out.push(LearnSample {
            line: i + 1,
            code,
            output,
        });
        i = end + 1;
    }
    out
}

/// Run one tutorial sample on `mode` (`--interpret` or `--native`): it must exit 0, print no
/// warning, and print exactly the output the page shows for it.
#[allow(dead_code)]
pub fn check_learn_sample(sample: &LearnSample, mode: &str, timeout: &str) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!(
        "loft_learn_{}_{}",
        std::process::id(),
        mode.trim_start_matches('-')
    ));
    fa::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = format!("learn_loft_line_{}.loft", sample.line);
    let path = dir.join(&name);
    fa::write(&path, &sample.code).map_err(|e| e.to_string())?;
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg(mode)
        .arg(&path)
        .env("LOFT_TIMEOUT", timeout)
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let _ = fa::remove_dir_all(&dir);
    let at = format!("doc/learn-loft.md:{} on {mode}", sample.line);
    if !out.status.success() {
        return Err(format!("{at} failed ({}):\n{stdout}{stderr}", out.status));
    }
    if loft_warnings(&stderr, &name) > 0 {
        return Err(format!("{at} warns:\n{stderr}"));
    }
    if let Some(expected) = &sample.output
        && stdout.trim_end() != expected.trim_end()
    {
        return Err(format!(
            "{at} printed\n{stdout}\nwhere the page shows\n{expected}"
        ));
    }
    Ok(())
}
