// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN140 — the profiling instruments, checked against programs whose answer is
//! known in advance.
//!
//! A profiler is unusually easy to ship broken, because every failure mode produces a
//! *plausible* report rather than an obviously empty one: a cache hit read as a
//! compile, a build read as a run, a one-sample symbol read as a hot spot. So nothing
//! here asserts "a report appeared". Each test states the answer first — this program
//! spends its time in `hot`, this one holds its memory at line 4, this one allocates
//! down two paths at 9:1 — and fails when the instrument says something else.
//!
//! Shares are asserted with generous floors rather than exact figures: the exact split
//! moves with the machine, and pinning it would make these change-detectors instead of
//! oracles. The floors are far enough from the wrong answer (naming a different
//! function, or missing a path entirely) that no amount of noise reaches them.
//!
//! The corpus-scale version of the same idea, over `bench/`, is
//! `scripts/profile_corpus.sh` and `bench/profile_oracle.tsv`.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Run `body` under the interpreter with `envs` set, returning stdout+stderr.
fn run(name: &str, body: &str, envs: &[(&str, &str)]) -> String {
    let path = std::env::temp_dir().join(format!("loft_prof_{name}.loft"));
    std::fs::write(&path, body).expect("write probe");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret").arg(&path);
    cmd.env("LOFT_TIMEOUT", "120");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    let _ = std::fs::remove_file(&path);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The first row of a `── <section> ──` block, which is the instrument's answer.
fn top_row<'a>(out: &'a str, section: &str) -> &'a str {
    out.lines()
        .skip_while(|l| !l.starts_with(section))
        .nth(1)
        .unwrap_or("")
}

/// `hot` runs ~100× the iterations of `cold`, so its share is not a matter of opinion.
const TWO_FUNCTIONS: &str = r#"
fn hot(n: integer) -> integer {
  acc = 0;
  for h_i in 0..n { acc += h_i % 7; }
  acc
}
fn cold(n: integer) -> integer {
  acc = 0;
  for c_i in 0..n { acc += c_i % 7; }
  acc
}
fn main() {
  t = hot(2000000) + cold(20000);
  println("result: {t}");
}
"#;

#[test]
fn cpu_profile_names_the_function_that_is_actually_hot() {
    let out = run("cpu_hot", TWO_FUNCTIONS, &[("LOFT_PROFILE", "1")]);
    let row = top_row(&out, "── by function");
    assert!(
        row.contains("hot") && !row.contains("cold"),
        "the hottest function is `hot` by two orders of magnitude.\nGot: {row}\n{out}"
    );
    // 100:1 in iterations; anything under half is the profiler measuring something
    // other than the work.
    let share: f64 = row
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    assert!(share > 50.0, "`hot` should dominate, got {share} %\n{out}");
}

#[test]
fn cpu_profile_names_the_hot_line_inside_it() {
    let out = run("cpu_line", TWO_FUNCTIONS, &[("LOFT_PROFILE", "1")]);
    let row = top_row(&out, "── by line");
    // The literal opens with a newline, so `fn hot` is line 2 and its loop body —
    // `for h_i in 0..n { acc += h_i % 7; }`, the only line executed two million
    // times — is line 4.
    assert!(
        row.contains(":4") && row.contains("hot"),
        "the hot line is the loop body of `hot` (line 4).\nGot: {row}\n{out}"
    );
}

/// A profiler that cannot name `main` names nothing: four out of five real programs
/// would also pass an instrument that simply reported the deepest frame.
#[test]
fn cpu_profile_names_main_when_main_is_the_work() {
    let out = run(
        "cpu_main",
        r#"
fn main() {
  acc = 0;
  for m_i in 0..2000000 { acc += m_i % 7; }
  println("result: {acc}");
}
"#,
        &[("LOFT_PROFILE", "1")],
    );
    let row = top_row(&out, "── by function");
    assert!(
        row.contains("main"),
        "with no helper, the whole run is `main`.\nGot: {row}\n{out}"
    );
}

/// The one thing a flat hot-spot table cannot fake.
#[test]
fn cpu_profile_shows_recursion_in_the_path() {
    let out = run(
        "cpu_path",
        r#"
fn fib(n: integer) -> integer {
  if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
}
fn main() { println("result: {fib(24)}"); }
"#,
        &[("LOFT_PROFILE", "1")],
    );
    let row = top_row(&out, "── hottest paths");
    assert!(
        row.matches("fib").count() >= 2,
        "`fib` is reached from `fib`; a path showing it only from `main` is not a path.\
         \nGot: {row}\n{out}"
    );
}

/// Off by default, and silent — a profile nobody asked for on stderr is a regression
/// in the tool, not a feature.
#[test]
fn nothing_is_reported_unless_it_was_asked_for() {
    let out = run("silent", TWO_FUNCTIONS, &[]);
    assert!(
        !out.contains("CPU profile") && !out.contains("allocation"),
        "an unarmed run must say nothing about profiling:\n{out}"
    );
}

/// The defect arc A exists to fix: the memory is held DURING the run and released
/// before exit, so an exit-triggered report sees an empty heap and says nothing.
#[test]
fn memory_report_is_taken_at_the_peak_not_at_exit() {
    let out = run(
        "mem_peak",
        r#"
fn build(n: integer) -> vector<integer> {
  return [for b_i in 0..n { b_i * 3 }];
}
fn main() {
  v = build(400000);
  println("result: {len(v)}");
}
"#,
        &[("LOFT_ALLOC_SITES", "1")],
    );
    assert!(
        out.contains("allocation hot spots"),
        "the report must appear:\n{out}"
    );
    let row = top_row(&out, "════ allocation hot spots");
    assert!(
        row.contains("build") && row.contains(":3"),
        "the heap was taken by `build` at line 3, and it is free again by exit — an \
         exit-time report would have nothing to say.\nGot: {row}\n{out}"
    );
    assert!(
        row.contains("MiB"),
        "400 000 integers is megabytes, and bytes are what the report is about \
         (store COUNTS would weigh this the same as one small record).\nGot: {row}"
    );
}

/// arc C — the case no `bench/` program can falsify: one allocation site, two paths,
/// a known ratio. A fixed-period sampler reported 100 % / 0 % here.
#[test]
fn allocation_paths_report_both_paths_at_their_true_ratio() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bench/profile_oracle/alloc_paths.loft");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&root)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_ALLOC_PATHS", "1")
        .output()
        .map(|o| {
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
        })
        .expect("spawn loft");
    let hot = out.contains("main → hot → make");
    let cold = out.contains("main → cold → make");
    assert!(
        hot && cold,
        "both paths to `make` must appear (hot: {hot}, cold: {cold}) — one path alone \
         is what a sampler in lock-step with the program reports.\n{out}"
    );
    // The rare path is 1 in 10; a sampler locked to the program's period reports it as
    // 0, and one that lost the site reports 50.
    let cold_n: u64 = out
        .lines()
        .zip(out.lines().skip(1))
        .find(|(_, next)| next.contains("main → cold → make"))
        .and_then(|(row, _)| {
            row.split_whitespace()
                .nth(2)
                .and_then(|s| s.parse::<u64>().ok())
        })
        .unwrap_or(0);
    let hot_n: u64 = out
        .lines()
        .zip(out.lines().skip(1))
        .find(|(_, next)| next.contains("main → hot → make"))
        .and_then(|(row, _)| {
            row.split_whitespace()
                .nth(2)
                .and_then(|s| s.parse::<u64>().ok())
        })
        .unwrap_or(0);
    assert!(cold_n > 0 && hot_n > 0, "both paths need samples\n{out}");
    let ratio = hot_n as f64 / cold_n as f64;
    assert!(
        (4.0..20.0).contains(&ratio),
        "the true ratio is 9:1; got {ratio:.1}:1 ({hot_n} / {cold_n})\n{out}"
    );
}

// ── loft#860: a test run is a loft workload, and was the one the sampler could not see ──

/// Two test files whose hot loops are BOTH on line 4, and whose iteration counts are
/// 4:1. The line collision is the point — see
/// [`merged_profile_keeps_each_row_s_own_file`].
const TEST_FILE_A: &str = r#"
fn work_a(n: integer) -> integer {
  acc = 0;
  for a_i in 0..n { acc += a_i % 7; }
  acc
}
fn test_a() { assert(work_a(800000) > 0); }
"#;

const TEST_FILE_B: &str = r#"
fn work_b(n: integer) -> integer {
  acc = 0;
  for b_i in 0..n { acc += b_i % 7; }
  acc
}
fn test_b() { assert(work_b(200000) > 0); }
"#;

/// Write the two-file suite into a fresh directory and run `--tests` over it.
fn run_tests_dir(name: &str, envs: &[(&str, &str)], extra_args: &[&str]) -> String {
    let dir = std::env::temp_dir().join(format!("loft_prof_tests_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create suite dir");
    std::fs::write(dir.join("a_file.loft"), TEST_FILE_A).expect("write a");
    std::fs::write(dir.join("b_file.loft"), TEST_FILE_B).expect("write b");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--tests");
    for a in extra_args {
        cmd.arg(a);
    }
    cmd.arg(&dir);
    cmd.env("LOFT_TIMEOUT", "120");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    let _ = std::fs::remove_dir_all(&dir);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// loft#860 — the report existed, the sampler worked, and a test run got neither:
/// `arm_profiler` had exactly one call site, on the program path. A suite is usually
/// the biggest interpreted workload a project owns, so this was the workload most
/// worth profiling and the only one that could not be.
#[test]
fn a_test_run_is_profiled() {
    let out = run_tests_dir("armed", &[("LOFT_PROFILE", "1")], &[]);
    assert!(
        out.contains("loft CPU profile"),
        "a test run must be profiled when LOFT_PROFILE asks for it — accepting the \
         variable and reporting nothing reads as 'the profiler found nothing'.\n{out}"
    );
    let row = top_row(&out, "── by function");
    assert!(
        row.contains("work_a"),
        "`work_a` runs 4× the iterations of `work_b`, so it is the answer.\nGot: {row}\n{out}"
    );
}

/// The merge has to happen on RESOLVED labels, not on bytecode positions: each test
/// compiles its own `Data`, so the same `pc` names different code in each one. Both
/// files here are hot on line 3, which is exactly the collision a `pc`-keyed merge
/// would fold into one row — and it would look like an ordinary profile.
#[test]
fn merged_profile_keeps_each_row_s_own_file() {
    let out = run_tests_dir("merged", &[("LOFT_PROFILE", "1")], &[]);
    assert!(
        out.contains("a_file.loft:4") && out.contains("b_file.loft:4"),
        "both files are hot at line 4 and must stay separate rows; merging raw \
         positions collapses them into one.\n{out}"
    );
    assert!(
        out.contains("across 2 runs"),
        "the banner must say how many runs it covers — 2 s over one run and 2 s over \
         420 invite different readings of the percentages.\n{out}"
    );
    // `work_a` runs 4× the iterations of `work_b`, so it must rank above it and
    // `work_b` must keep a real share. Only the ORDER and a floor are asserted, not
    // the split: the run that goes first also pays the run-once costs (page faults,
    // first touch of the stdlib), so the measured ratio sits above the iteration
    // ratio — around 7:1 here — and pinning it would make this a change-detector.
    // The floor is what matters: a dropped run reads as a function that went quiet.
    let share = |name: &str| -> f64 {
        out.lines()
            .find(|l| l.contains(name) && l.contains('%'))
            .and_then(|l| l.split_whitespace().next().and_then(|s| s.parse().ok()))
            .unwrap_or(0.0)
    };
    let (a, b) = (share("work_a"), share("work_b"));
    assert!(
        a > b && b > 2.0,
        "both runs must survive the merge (work_a {a} %, work_b {b} %) — a lost run \
         reads as a quiet function.\n{out}"
    );
}

/// Still off by default. The suite runs 3931 tests; a profile on stderr for every
/// unprofiled one would be the loudest regression in the tool.
#[test]
fn a_test_run_is_silent_unless_asked() {
    let out = run_tests_dir("quiet", &[], &[]);
    assert!(
        !out.contains("CPU profile") && !out.contains("allocation paths"),
        "an unarmed test run must say nothing about profiling:\n{out}"
    );
}

/// The heap ledger ranks a process-wide peak by bytecode position, and a suite's peak
/// may be reached in any of its runs — each of which compiled its own bytecode. So it
/// REFUSES rather than resolving those positions against whichever run finished last,
/// which would name real lines in the wrong file.
#[test]
fn alloc_sites_refuses_a_test_run_instead_of_going_quiet() {
    let out = run_tests_dir("sites", &[("LOFT_ALLOC_SITES", "1")], &[]);
    assert!(
        out.contains("LOFT_ALLOC_SITES is not available under a test run"),
        "an instrument that cannot answer must say so — silence reads as 'nothing to \
         report'.\n{out}"
    );
    assert!(
        !out.contains("allocation hot spots"),
        "…and it must not print a table it cannot attribute:\n{out}"
    );
}

// ── loft#865 and its neighbour: the two ways a profile can be silent or blind ──

/// loft#865 — the DEFAULT backend is native, so this is the run a user reaches for a
/// profiler WITH, and it accepted `LOFT_PROFILE` and exited 0 with an empty terminal.
/// That is indistinguishable from "the profiler ran and your program is not the
/// problem". `--interpret` was the only arm that armed the sampler.
///
/// Ignored by default: it needs rustc to build the native binary, which the ordinary
/// suite does not assume.
#[test]
fn a_native_run_says_the_sampler_cannot_follow_it() {
    let path = std::env::temp_dir().join("loft_prof_865.loft");
    std::fs::write(&path, "fn main() { println(\"x\"); }\n").expect("write probe");
    // Both spellings: the explicit flag, and the DEFAULT, which is the same backend
    // reached without typing anything. They were silent for the identical reason, so
    // fixing only the explicit one would leave the common case broken.
    for args in [vec!["--native"], vec![]] {
        let mut cmd = Command::new(loft_bin());
        for a in &args {
            cmd.arg(a);
        }
        let out = cmd
            .arg(&path)
            .env("LOFT_PROFILE", "1")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("spawn loft");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            text.contains("interpreter-only") && text.contains("Add --interpret"),
            "a native run must say the variable was read and cannot be honoured, and \
             name the cure (args: {args:?}):\n{text}"
        );
    }
    let _ = std::fs::remove_file(&path);
}

/// The blind spot moros measured, and the reason it is worse than silence: the report
/// is POPULATED and inverted. A `use`d library runs as a compiled cdylib, so its
/// functions cannot be sampled — their time lands on the calling line.
///
/// The probe is built so the true answer is not a matter of opinion: the library loops
/// 150× what the program does, so it owns ~99 % of the run. Unwarned, the table said
/// `100 % app_bit`.
#[test]
fn a_profile_says_when_a_used_library_is_invisible_to_it() {
    let root = std::env::temp_dir().join("loft_prof_libblind");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("hotlib/src")).expect("lib dir");
    std::fs::create_dir_all(root.join("app")).expect("app dir");
    std::fs::write(
        root.join("hotlib/loft.toml"),
        "[package]\nname = \"hotlib\"\nversion = \"0.0.0\"\nloft = \">=0.8\"\n\n\
         [library]\nentry = \"src/hotlib.loft\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("hotlib/src/hotlib.loft"),
        "pub fn lib_grind(n: integer) -> integer {\n  acc = 0;\n  \
         for i in 0..n { acc = acc + i % 7; }\n  acc\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("app/prog.loft"),
        "use hotlib::*;\n\nfn app_bit(n: integer) -> integer {\n  t = 0;\n  \
         for i in 0..n { t = t + i % 3; }\n  t\n}\n\n\
         fn main() {\n  a = lib_grind(3000000);\n  b = app_bit(20000);\n  \
         println(\"{a} {b}\");\n}\n",
    )
    .unwrap();

    let run = |no_native: bool| -> String {
        let mut cmd = Command::new(loft_bin());
        cmd.arg("--interpret")
            .arg("--lib")
            .arg(&root)
            .arg(root.join("app/prog.loft"))
            .env("LOFT_PROFILE", "1")
            .env("LOFT_TIMEOUT", "180");
        if no_native {
            cmd.env("LOFT_NO_NATIVE_LIBS", "1");
        }
        let o = cmd.output().expect("spawn loft");
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    };

    let blind = run(false);
    assert!(
        blind.contains("CALLED INTO `use`d LIBRARIES"),
        "the report must announce the blind spot before its tables:\n{blind}"
    );
    assert!(
        blind.contains("LOFT_NO_NATIVE_LIBS=1"),
        "…and name the switch that lifts it:\n{blind}"
    );

    // The claim is not decoration: with the libraries interpreted, the ranking really
    // does invert. If this ever stops inverting, the warning has become a lie and
    // should be reworded, not deleted.
    let seeing = run(true);
    let top = top_row(&seeing, "── by function");
    assert!(
        top.contains("lib_grind"),
        "with LOFT_NO_NATIVE_LIBS=1 the library must dominate — it loops 150× the \
         program's work.\nGot: {top}\n{seeing}"
    );
    let blind_top = top_row(&blind, "── by function");
    assert!(
        blind_top.contains("app_bit"),
        "…and without it, the CALLER is what the table shows, which is the whole \
         hazard.\nGot: {blind_top}\n{blind}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A program that never finishes on its own — a server's shape, without the sockets.
///
/// `LOFT_TIMEOUT` stops it, which is the runner's own watchdog and not an exit the
/// program reaches: that is the point, because the report used to render at process exit
/// and a run with no exit therefore produced none.
const RUNS_UNTIL_STOPPED: &str = r#"
fn grind(n: integer) -> integer {
  acc = 0;
  for g_i in 0..n { acc += g_i % 7; }
  acc
}
fn main() {
  t = 0;
  for round in 0..1000000 { t += grind(50000); }
  println("never reached: {t}");
}
"#;

/// loft#1089 — a program with no clean shutdown can still be profiled.
///
/// The report renders at process exit, so the one class of program you most want a
/// profile of — a server under load, whose only exit is the operator's `kill` — was the
/// class that could not give you one. A periodic flush needs no signal and no exit: what
/// was already printed is already out, so it survives a hard kill too.
///
/// The assertion is on the COUNT of reports rather than on their presence, because one
/// report is what an ordinary exit already produced: what has to be true here is that
/// the program reported WHILE RUNNING, more than once, without ever reaching its end.
#[test]
fn a_program_that_never_exits_still_reports_its_profile() {
    let path = std::env::temp_dir().join("loft_prof_periodic.loft");
    std::fs::write(&path, RUNS_UNTIL_STOPPED).expect("write probe");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .env("LOFT_PROFILE", "1")
        .env("LOFT_PROFILE_EVERY", "1")
        // Stopped by the watchdog, not by the program: it has no end of its own.
        .env("LOFT_TIMEOUT", "5")
        .output()
        .expect("spawn loft");
    let _ = std::fs::remove_file(&path);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let reports = text.matches("loft CPU profile").count();
    assert!(
        reports >= 2,
        "a one-second flush over a five-second run must report more than once — and the \
         program never reaches its own end, so every one of them came from the flush.\n\
         Got {reports}:\n{text}"
    );
    assert!(
        !text.contains("never reached"),
        "the probe must not finish, or the reports could be ordinary exit-time ones.\n{text}"
    );
    // The rows have to be a real answer, not an empty banner: `grind` is where every
    // sample of this program belongs.
    assert!(
        top_row(&text, "── by function").contains("grind"),
        "a mid-run report must carry the same rows an exit-time one does.\n{text}"
    );
}

/// The other half of the same claim: an UNPROFILED run's shutdown is untouched.
///
/// The signal handlers are installed only when the profiler arms, because a handler that
/// absorbs `SIGINT` would change what Ctrl-C does for every program — a much larger
/// change than the one being made, and one nobody asked for.
#[test]
fn an_unprofiled_run_installs_no_signal_handlers() {
    let path = std::env::temp_dir().join("loft_prof_no_handlers.loft");
    std::fs::write(&path, RUNS_UNTIL_STOPPED).expect("write probe");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .env("LOFT_TIMEOUT", "3")
        .env_remove("LOFT_PROFILE")
        .env_remove("LOFT_PROFILE_EVERY")
        .env_remove("LOFT_ALLOC_PATHS")
        .output()
        .expect("spawn loft");
    let _ = std::fs::remove_file(&path);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !text.contains("loft CPU profile"),
        "an unarmed run reports nothing at all.\n{text}"
    );
}

/// loft#1088 — `LOFT_NET_PROFILE=1` accumulated every event into a summary that nothing
/// ever printed: only `=trace`, which prints per event, produced output at all.
///
/// So "no report" was the answer for every program, and a consumer arming it against a
/// socket server spent an investigation on it — because a report that never prints and
/// an instrument that sees nothing look exactly the same from outside.
///
/// The empty case is the one asserted here, since it is the one a consumer hits: armed,
/// nothing recorded, and the report SAYS so and names what it can see. A silent
/// instrument and a broken one are indistinguishable, which is the same lesson the CPU
/// profiler learned from a `--native` run (loft#865).
#[test]
fn an_armed_network_profile_says_so_when_it_recorded_nothing() {
    let out = run(
        "net_empty",
        "fn main() { println(\"no sockets here\"); }",
        &[("LOFT_NET_PROFILE", "1")],
    );
    assert!(
        out.contains("[net-profile] armed, and no socket operation was recorded"),
        "an armed run that touched no socket must say so rather than print nothing:\n{out}"
    );
    assert!(
        out.contains("the sockets the RUNTIME owns"),
        "…and name its reach, because the reader's next question is whether the switch \
         works:\n{out}"
    );
    assert!(
        out.contains("net_profile::time"),
        "…and name how a library joins the report, which is the actual cure for a \
         program built on one:\n{out}"
    );
}

/// The control: an unarmed run says nothing at all about the network.
#[test]
fn an_unarmed_network_profile_is_silent() {
    let out = run(
        "net_off",
        "fn main() { println(\"no sockets here\"); }",
        &[],
    );
    assert!(
        !out.contains("[net-profile]"),
        "an unarmed run must not mention the instrument:\n{out}"
    );
}
