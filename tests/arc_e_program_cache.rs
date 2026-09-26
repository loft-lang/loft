// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN11 arc E — the opt-in whole-program startup cache, end-to-end.
//!
//! On `LOFT_PROGRAM_CACHE` a cold run caches the ENTIRE parsed program (stdlib +
//! the script's lazily-loaded libs + user file) keyed on the script path, and a
//! warm run mmaps it and skips ALL parsing.  A drift manifest of every parsed
//! source's content hash invalidates the bundle whenever any input changes.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run the binary on `script`, optionally with the whole-program cache enabled
/// at `cache_dir` (`XDG_CACHE_HOME`).  Returns `(success, stdout)`.
fn run(script: &std::path::Path, cache_dir: Option<&std::path::Path>) -> (bool, String) {
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret")
        .arg(script)
        .current_dir(workspace_root())
        .env_remove("LOFT_STDLIB_CACHE");
    if let Some(dir) = cache_dir {
        cmd.env("LOFT_PROGRAM_CACHE", "1")
            .env("XDG_CACHE_HOME", dir);
    } else {
        cmd.env_remove("LOFT_PROGRAM_CACHE");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn program_cache_cold_warm_then_drift() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_arce_{pid}.loft"));
    let write = |body: &str| std::fs::write(&script, body).expect("write script");
    write("fn main() {\n  v = [5, 10, 15];\n  print(\"sum={v[0]+v[1]+v[2]}\\n\");\n}\n");
    let cache_dir = tmp.join(format!("loft_arce_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);

    // 1. Cache off.
    let (ok_off, out_off) = run(&script, None);
    assert!(ok_off, "cache-off run failed: {out_off}");
    assert!(out_off.contains("sum=30"), "off output: {out_off}");

    // 2. Cold — parses then writes a whole-program bundle + manifest.
    let (ok_cold, out_cold) = run(&script, Some(&cache_dir));
    assert!(ok_cold, "cold run failed: {out_cold}");
    let dir = cache_dir.join("loft");
    let has = |ext: &str| {
        std::fs::read_dir(&dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some(ext))
    };
    assert!(
        has("store") && has("manifest"),
        "cold run must write bundle + manifest"
    );

    // 3. Warm — mmaps the whole program, skips all parsing.
    let (ok_warm, out_warm) = run(&script, Some(&cache_dir));
    assert!(ok_warm, "warm run failed: {out_warm}");
    assert_eq!(out_off, out_cold, "cold differs from off");
    assert_eq!(out_off, out_warm, "warm differs from off");

    // 4. Drift — edit the script; the manifest hash mismatches → reparse, new output.
    write("fn main() {\n  v = [5, 10, 100];\n  print(\"sum={v[0]+v[1]+v[2]}\\n\");\n}\n");
    let (ok_drift, out_drift) = run(&script, Some(&cache_dir));
    assert!(ok_drift, "drift run failed: {out_drift}");
    assert!(
        out_drift.contains("sum=115"),
        "drift must reparse → 115, got: {out_drift}"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache_dir);
}

/// #358 — a no-`main` script (the zero-param test-fn fallback) must execute on
/// WARM cache loads too.  The fallback iterates `start_def..definitions()`, and
/// a warm load restores stdlib + user defs in one table before `start_def` is
/// taken — pre-fix the range was empty, so the first run worked and every later
/// run was a silent no-op (exit 0, no output).  The fix persists the boundary
/// as a `udef` manifest line and replays it on warm loads.
#[test]
fn no_main_script_executes_on_warm_load() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_358_{pid}.loft"));
    std::fs::write(
        &script,
        "fn check_me() {\n    println(\"hello from check_me\");\n}\n",
    )
    .expect("write script");
    let cache_dir = tmp.join(format!("loft_358_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);

    // Cold run parses, executes the fallback, and writes the bundle.
    let (ok_cold, out_cold) = run(&script, Some(&cache_dir));
    assert!(ok_cold, "cold run failed: {out_cold}");
    assert!(
        out_cold.contains("hello from check_me"),
        "cold output: {out_cold}"
    );

    // Warm runs must produce the same output — pre-fix they were silent no-ops.
    for nth in ["first", "second"] {
        let (ok_warm, out_warm) = run(&script, Some(&cache_dir));
        assert!(ok_warm, "{nth} warm run failed: {out_warm}");
        assert_eq!(
            out_cold, out_warm,
            "{nth} warm run of a no-main script must execute the test-fn fallback (#358)"
        );
    }

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache_dir);
}

/// @PLN11 G2/M6 — the manifest's build-signature line invalidates a stale
/// bundle on a (simulated) binary upgrade.  Without it, a bundle written by an
/// older build would be warm-loaded by a newer one whose store layout/codegen
/// differs — a silent stale-load.  We simulate the upgrade by rewriting the
/// manifest's `sig ` line; the next run must reparse (cache miss), not load the
/// stale bundle, and still produce correct output.
#[test]
fn manifest_build_signature_invalidates_stale_bundle() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_sig_{pid}.loft"));
    std::fs::write(&script, "fn main() { print(\"answer={6*7}\\n\"); }\n").expect("write script");
    let cache_dir = tmp.join(format!("loft_sig_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);

    // Cold run primes the bundle + a manifest whose first line is `sig <build>`.
    let (ok_cold, _) = run(&script, Some(&cache_dir));
    assert!(ok_cold, "cold run failed");
    let dir = cache_dir.join("loft");
    let manifest = std::fs::read_dir(&dir)
        .expect("cache dir")
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|x| x.to_str()) == Some("manifest"))
        .expect("manifest written");
    let text = std::fs::read_to_string(&manifest).expect("read manifest");
    assert!(
        text.starts_with("sig "),
        "manifest must start with the sig line: {text:?}"
    );

    // Simulate a binary upgrade: clobber the sig line with a different build.
    let body: String = text.lines().skip(1).map(|l| format!("{l}\n")).collect();
    std::fs::write(&manifest, format!("sig STALE-OTHER-BUILD\n{body}")).expect("tamper manifest");

    // Next run must treat it as a cache miss (reparse) and still be correct —
    // NOT warm-load the now-"foreign" bundle.
    let (ok, out) = run(&script, Some(&cache_dir));
    assert!(ok, "run after sig mismatch failed: {out}");
    assert!(
        out.contains("answer=42"),
        "must reparse to correct output: {out}"
    );

    // And it should have re-saved a manifest with THIS build's real signature.
    let restored = std::fs::read_to_string(&manifest).expect("read manifest");
    assert!(
        !restored.contains("STALE-OTHER-BUILD"),
        "a cold reparse should overwrite the stale-sig manifest"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache_dir);
}

/// @PLN11 — regression: a `#cwd` program's parse-time path-resolution mode must
/// survive a warm load.  `#cwd` flips `program_relative` to false (cwd-relative)
/// at parse time; a warm load skips parsing, so without the manifest's `prel`
/// line the mode reverted to the program-relative default and `file()` resolved
/// against the program's own dir — the bug that made the indexer scan nothing on
/// a cached run.  The program checks `Cargo.toml` relative to cwd (= workspace
/// root, set by `run`): cwd-relative finds it (`found=true`), program-relative
/// looks in the script's /tmp dir (`found=false`).
#[test]
fn cwd_directive_survives_warm_load() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_cwd_{pid}.loft"));
    std::fs::write(
        &script,
        "#cwd\nfn main() {\n  found = file(\"Cargo.toml\").format != Format.NotExists;\n  print(\"found={found}\\n\");\n}\n",
    )
    .expect("write script");
    let cache_dir = tmp.join(format!("loft_cwd_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);

    // Cache-off baseline: `#cwd` resolves cwd-relative → finds the repo Cargo.toml.
    let (ok_off, out_off) = run(&script, None);
    assert!(ok_off, "cache-off failed: {out_off}");
    assert!(
        out_off.contains("found=true"),
        "#cwd must resolve cwd-relative (found Cargo.toml): {out_off}"
    );

    // Cold parses the `#cwd` directive; warm skips parsing and must restore the mode.
    let (ok_cold, out_cold) = run(&script, Some(&cache_dir));
    assert!(ok_cold, "cold failed: {out_cold}");
    let (ok_warm, out_warm) = run(&script, Some(&cache_dir));
    assert!(ok_warm, "warm failed: {out_warm}");
    assert_eq!(out_off, out_cold, "cold differs from cache-off");
    // The bug: warm reverted to program-relative → looked in /tmp → `found=false`.
    assert_eq!(
        out_off, out_warm,
        "warm lost the #cwd mode (program-relative regression): {out_warm}"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache_dir);
}

/// #310 — a program whose library carries a native cdylib (`[library]
/// native = "<stem>"`) must dlopen it on WARM runs too.  The registration
/// (`pending_native_libs`) is parse-time state, and the warm load skips
/// parsing — pre-fix it received an empty list, left the panic stubs wired,
/// and every cached run died at the first `#native` call ("native function
/// not loaded").  The fix persists `nlib <stem> <pkg_dir>` manifest lines and
/// re-resolves them at warm load (cold-equal freshness semantics).
#[test]
fn program_cache_warm_keeps_native_libs() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_arce_nlib_{pid}.loft"));
    std::fs::write(
        &script,
        "use native_pkg;\n\nfn main() {\n    r = ext_add_one(41);\n    print(\"r={r}\\n\");\n}\n",
    )
    .expect("write script");
    let cache_dir = tmp.join(format!("loft_arce_nlib_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);

    let run_lib = |cache: Option<&std::path::Path>| {
        let mut cmd = Command::new(loft_bin());
        cmd.arg("--interpret")
            .arg("--lib")
            .arg(workspace_root().join("tests/lib"))
            .arg(&script)
            .current_dir(workspace_root())
            .env_remove("LOFT_STDLIB_CACHE");
        if let Some(dir) = cache {
            cmd.env("LOFT_PROGRAM_CACHE", "1")
                .env("XDG_CACHE_HOME", dir);
        } else {
            cmd.env_remove("LOFT_PROGRAM_CACHE");
        }
        let out = cmd.output().expect("failed to invoke loft binary");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    // Cache-off baseline (also auto-builds the fixture cdylib if needed).
    let (ok_off, out_off, err_off) = run_lib(None);
    assert!(ok_off, "cache-off failed: {out_off}\n{err_off}");
    assert!(out_off.contains("r=42"), "off output: {out_off}");

    // Cold writes the bundle + manifest (with the nlib registration).
    let (ok_cold, out_cold, err_cold) = run_lib(Some(&cache_dir));
    assert!(ok_cold, "cold failed: {out_cold}\n{err_cold}");
    assert!(out_cold.contains("r=42"), "cold output: {out_cold}");

    // Warm must dlopen the cdylib again — pre-#310 this panicked with
    // "native function not loaded".
    let (ok_warm, out_warm, err_warm) = run_lib(Some(&cache_dir));
    assert!(
        ok_warm,
        "warm run lost the native-lib registration (#310): {out_warm}\n{err_warm}"
    );
    assert!(out_warm.contains("r=42"), "warm output: {out_warm}");

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache_dir);
}

/// #322 — a `--lib`-resolved dependency edit must invalidate the program
/// bundle.  Library files are parsed via an inline lexer switch (never a
/// `parse()` entry), so before the fix `parsed_sources` — and therefore the
/// drift manifest — missed them: the edited library's old bytecode kept
/// executing until the cache was deleted by hand.
#[test]
fn lib_dependency_edit_invalidates_program_cache() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let lib_dir = tmp.join(format!("loft_p322_lib_{pid}"));
    let _ = std::fs::remove_dir_all(&lib_dir);
    std::fs::create_dir_all(&lib_dir).expect("mkdir lib");
    let lib_file = lib_dir.join("rstream.loft");
    let write_lib = |body: &str| std::fs::write(&lib_file, body).expect("write lib");
    write_lib("pub fn stream_value() -> integer { 28 }\n");
    let script = tmp.join(format!("loft_p322_main_{pid}.loft"));
    std::fs::write(
        &script,
        "use rstream;\nfn main() { print(\"v={stream_value()}\\n\"); }\n",
    )
    .expect("write script");
    let cache_dir = tmp.join(format!("loft_p322_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);

    let run_lib = |label: &str| -> String {
        let out = Command::new(loft_bin())
            .arg("--interpret")
            .arg("--lib")
            .arg(&lib_dir)
            .arg(&script)
            .current_dir(workspace_root())
            .env_remove("LOFT_STDLIB_CACHE")
            .env("LOFT_PROGRAM_CACHE", "1")
            .env("XDG_CACHE_HOME", &cache_dir)
            .output()
            .expect("failed to invoke loft binary");
        assert!(out.status.success(), "{label} run failed");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    // Cold run caches; the manifest must cover the lib file.
    let out_cold = run_lib("cold");
    assert!(out_cold.contains("v=28"), "cold output: {out_cold}");
    let manifest = std::fs::read_dir(cache_dir.join("loft"))
        .expect("cache dir")
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().ends_with(".manifest"))
        .expect("manifest written");
    let manifest_text = std::fs::read_to_string(manifest.path()).expect("read manifest");
    assert!(
        manifest_text.contains("rstream.loft"),
        "manifest must list the --lib dependency:\n{manifest_text}"
    );

    // Edit the LIBRARY only — the warm run must see the new behaviour.
    write_lib("pub fn stream_value() -> integer { 777 }\n");
    let out_edited = run_lib("post-edit");
    assert!(
        out_edited.contains("v=777"),
        "stale cache served after lib edit: {out_edited}"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&lib_dir);
    let _ = std::fs::remove_dir_all(&cache_dir);
}

/// A warm run renders the SAME diagnostics a cold one does — including the parts computed
/// from a diagnostic's `fixes`.
///
/// Diagnostics are a parser product, so a bundle that skips the parser has to carry them.
/// Carrying only the message is not enough, and the two things it misses are both things a
/// normal run prints:
///
///   * the once-per-run *"N diagnostics above suggest what to write instead"* note, which
///     counts entries whose `fixes` are non-empty — a warm run dropped the line entirely;
///   * every `fix` line under `--explain`, which reads the cache like any other run rather
///     than forcing a cold parse as its design assumed — two lines cold, none warm.
///
/// Asserted as EQUALITY between the two runs plus a positive check that the cold run
/// actually produced the thing being compared: two empty stderrs are equal too.
#[test]
fn a_warm_run_renders_the_same_diagnostics_including_their_fixes() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_diagfix_{pid}.loft"));
    let cache = tmp.join(format!("loft_diagfix_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&cache).expect("create cache dir");
    // `omitted-field-zero` is an ADVICE that carries two fixes, one of them with an edit.
    std::fs::write(
        &script,
        "struct DfPlayer { name: text, health: integer }\nfn main() {\n  p = DfPlayer { name: \"Bob\" };\n  print(\"{p.name}\");\n}\n",
    )
    .expect("write script");

    let stderr_of = |explain: bool| -> String {
        let mut cmd = Command::new(loft_bin());
        cmd.arg("--interpret")
            .arg(&script)
            .current_dir(workspace_root())
            .env_remove("LOFT_STDLIB_CACHE")
            .env("LOFT_PROGRAM_CACHE", "1")
            .env("XDG_CACHE_HOME", &cache);
        if explain {
            cmd.env("LOFT_EXPLAIN", "1");
        }
        let out = cmd.output().expect("failed to invoke loft binary");
        String::from_utf8_lossy(&out.stderr).into_owned()
    };

    let cold = stderr_of(false);
    let warm = stderr_of(false);
    assert!(
        cold.contains("omitted-field-zero"),
        "the cold run must produce the advice this compares; got: {cold}"
    );
    assert!(
        cold.contains("suggests what to write instead"),
        "the cold run must produce the fixes-derived note; got: {cold}"
    );
    assert_eq!(
        cold, warm,
        "a warm run must render what the cold run rendered"
    );

    // `--explain` prints the fix lines themselves.  Fresh cache dir so the first of the two
    // is genuinely cold for this mode as well.
    let cache_x = tmp.join(format!("loft_diagfix_cache_x_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_x);
    std::fs::create_dir_all(&cache_x).expect("create cache dir");
    let stderr_x = |dir: &std::path::Path| -> String {
        let out = Command::new(loft_bin())
            .arg("--interpret")
            .arg(&script)
            .current_dir(workspace_root())
            .env_remove("LOFT_STDLIB_CACHE")
            .env("LOFT_PROGRAM_CACHE", "1")
            .env("XDG_CACHE_HOME", dir)
            .env("LOFT_EXPLAIN", "1")
            .output()
            .expect("failed to invoke loft binary");
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let cold_x = stderr_x(&cache_x);
    let warm_x = stderr_x(&cache_x);
    assert_eq!(
        cold_x.matches("  fix ").count(),
        2,
        "the cold --explain run must print both fixes; got: {cold_x}"
    );
    assert_eq!(
        cold_x, warm_x,
        "a warm --explain run must print the fix lines a cold one printed"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache);
    let _ = std::fs::remove_dir_all(&cache_x);
}
/// `introspect` always PARSES: a warm bundle carries no variable table, so under a cache hit
/// the dump rendered every variable as `name(65535)` and the slot table's number and span
/// columns as `-` — two runs of one binary read as two different compilers, which is the
/// exact question `scripts/introspect_diff.sh` asks.  Measured on the released 2026.8.0.
#[test]
fn introspect_parses_fresh_under_a_warm_program_cache() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_arce_introspect_{pid}.loft"));
    std::fs::write(
        &script,
        "fn main() {\n  total = 0;\n  for n in 1..4 { total = total + n; }\n  print(\"{total}\\n\");\n}\n",
    )
    .expect("write script");
    let cache_dir = tmp.join(format!("loft_arce_introspect_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);
    let introspect = || {
        let out = Command::new(loft_bin())
            .arg("introspect")
            .arg(&script)
            .current_dir(workspace_root())
            .env_remove("LOFT_STDLIB_CACHE")
            .env("LOFT_PROGRAM_CACHE", "1")
            .env("XDG_CACHE_HOME", &cache_dir)
            .output()
            .expect("failed to invoke loft binary");
        assert!(
            out.status.success(),
            "introspect failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // Warm the cache the way an ordinary run does, so a bundle EXISTS for the script.
    let (ok, _) = run(&script, Some(&cache_dir));
    assert!(ok, "the warming run failed");
    let first = introspect();
    let second = introspect();
    assert!(
        first.contains("total(") && !first.contains("(65535)"),
        "the dump must carry real variable numbers:\n{first}"
    );
    assert_eq!(
        first, second,
        "two introspect runs of one binary must emit identically"
    );
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache_dir);
}

/// `@FR-O-Witness` — a heap-record local whose assignments MIX ownership releases its
/// stores through an owner witness both emitters READ (`Function::owner_witness`) to copy
/// into a FRESH store rather than in place.  The witness was maintained in the IR and
/// restored by no snapshot field, so a WARM run — the parse replaced by the cached bundle —
/// emitted the pre-witness copy arm and wrote the copy INTO the record the local was
/// viewing: the sharp cell of loft#1336 answered `b == 7` on its second run and `b == 2`
/// on its first, on both backends.  A fact the emitters read must survive the snapshot
/// exactly as `skip_free` does; this pins that for the witness, cold against warm.
#[test]
fn a_warm_run_keeps_the_owner_witness() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_arce_witness_{pid}.loft"));
    std::fs::write(
        &script,
        "struct Node { value: integer, next: reference<Node>? }\n\
         fn run() -> integer {\n\
         \x20 c = Node { value: 4, next: null };\n\
         \x20 b = Node { value: 2, next: c };\n\
         \x20 a = Node { value: 1, next: b };\n\
         \x20 s: Node? = a;\n\
         \x20 s = a.next;\n\
         \x20 s = a;\n\
         \x20 s.value = 7;\n\
         \x20 b.value * 100 + a.value * 10 + s.value\n\
         }\n\
         fn main() { println(\"witness {run()}\"); }\n",
    )
    .expect("write script");
    let cache_dir = tmp.join(format!("loft_arce_witness_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);
    for backend in ["--interpret", "--native"] {
        let run = |warm_label: &str| -> String {
            let out = Command::new(loft_bin())
                .arg(backend)
                .arg(&script)
                .current_dir(workspace_root())
                .env_remove("LOFT_STDLIB_CACHE")
                .env("LOFT_PROGRAM_CACHE", "1")
                .env("LOFT_STRICT_STORES", "1")
                .env("XDG_CACHE_HOME", &cache_dir)
                .output()
                .expect("failed to invoke loft binary");
            assert!(
                out.status.success(),
                "{backend} {warm_label} run failed: {}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        let cold = run("cold");
        assert!(
            cold.contains("witness 217"),
            "{backend} cold output: {cold}"
        );
        let warm = run("warm");
        assert!(
            warm.contains("witness 217"),
            "{backend} warm run copied into the viewed record (b overwritten): {warm}"
        );
    }
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache_dir);
}

/// A per-variable fact the emitters read must survive the warm load, which is a second
/// decoder of the parsed program.  A narrow local a `&` names holds its type's FIELD
/// encoding (@PLN167 decision 1); the snapshot did not carry that fact, so a warm run read
/// the encoded slot as the value — `-135` for the `-7` the cold run printed — with no
/// diagnostic.  `i8` and a `limit` range both carry a non-zero bias, which is what makes
/// the encoding differ from the value (`u8`'s is zero and reads the same either way).
#[test]
fn a_linked_narrow_local_reads_the_same_warm() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_linked_narrow_{pid}.loft"));
    std::fs::write(
        &script,
        "fn main() {\n  x: i8 = -1;\n  p = &x;\n  p = -7;\n  h: integer limit(1000, 1100) = 1050;\n  q = &h;\n  q = 1020;\n  println(\"{x} {p} {h} {q}\");\n}\n",
    )
    .expect("write script");
    let cache_dir = tmp.join(format!("loft_linked_narrow_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);
    let (ok_cold, out_cold) = run(&script, Some(&cache_dir));
    assert!(ok_cold, "cold run failed: {out_cold}");
    assert_eq!(out_cold.trim(), "-7 -7 1020 1020", "cold output");
    for nth in ["first", "second"] {
        let (ok_warm, out_warm) = run(&script, Some(&cache_dir));
        assert!(ok_warm, "{nth} warm run failed: {out_warm}");
        assert_eq!(
            out_warm, out_cold,
            "{nth} warm run of a linked narrow local"
        );
    }
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache_dir);
}

/// @PLN167 C3 — a `&text` parameter handed a text field is served by the function's STORE
/// instance: a definition minted after pass 2 whose parameter carries `store_text_link`.  Both
/// the minted definition and that per-variable fact must survive the warm load, or a warm run
/// calls the stack instance with a slot reference and reads garbage.  Forwarding (`fwd` calls
/// `app`) makes the instances close transitively, so two minted definitions ride the bundle,
/// and a call through a function value (loft#1656) dispatches to one by the call's mask.
#[test]
fn a_store_text_instance_reads_the_same_warm() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_store_text_{pid}.loft"));
    std::fs::write(
        &script,
        "struct O { a: text, b: text }\nfn app(t: &text, k: integer) { t += \"!{k}\"; }\nfn fwd(t: &text) { app(t, 7); t += \".\"; }\nfn main() {\n  o = O { a: \"alpha\", b: \"beta\" };\n  fwd(o.b);\n  v: vector<text> = [\"aa\", \"bb\"];\n  app(v[1], 2);\n  s = \"x\";\n  fwd(s);\n  g = app;\n  g(o.a, 9);\n  println(\"{o.a} {o.b} {v} {s}\");\n}\n",
    )
    .expect("write script");
    let cache_dir = tmp.join(format!("loft_store_text_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);
    let (ok_cold, out_cold) = run(&script, Some(&cache_dir));
    assert!(ok_cold, "cold run failed: {out_cold}");
    assert_eq!(
        out_cold.trim(),
        "alpha!9 beta!7. [\"aa\",\"bb!2\"] x!7.",
        "cold output"
    );
    for nth in ["first", "second"] {
        let (ok_warm, out_warm) = run(&script, Some(&cache_dir));
        assert!(ok_warm, "{nth} warm run failed: {out_warm}");
        assert_eq!(
            out_warm, out_cold,
            "{nth} warm run of a store text instance"
        );
    }
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache_dir);
}

/// A stdlib that changed under a cached program is never served stale (@PLN166 B1).
///
/// The program bundle holds the parsed stdlib, and on a program-cache miss the stdlib
/// itself may come from its own cache.  Either one read after `default/` changed would
/// answer with the OLD library, and nothing would say so.  Each cell works on a scratch
/// copy of `default/` (passed with `--path`), warms both caches, changes the library, and
/// requires the next run to see the change: an EDITED function returns its new value, an
/// ADDED file's function resolves, a REMOVED file's function is refused.
fn run_with_stdlib(
    script: &std::path::Path,
    root: &std::path::Path,
    cache_dir: &std::path::Path,
) -> (bool, String) {
    let out = Command::new(loft_bin())
        .arg("--path")
        .arg(root)
        .arg("--interpret")
        .arg(script)
        .current_dir(workspace_root())
        .env_remove("LOFT_STDLIB_CACHE")
        .env_remove("LOFT_NO_CACHE")
        .env("LOFT_PROGRAM_CACHE", "1")
        .env("XDG_CACHE_HOME", cache_dir)
        .output()
        .expect("failed to invoke loft binary");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// Warm both caches the way the edit loop does.  The first run parses the stdlib and writes
/// its cache; the program is then EDITED, so the second run misses the program cache and
/// takes the stdlib from its own cache — the bundle it writes is the one that carries no
/// stdlib sources of its own; the third run is served from that bundle.  Without the edit
/// the bundle comes from a run that parsed the stdlib, and a cell never reaches the case.
fn warm_through_an_edit(script: &std::path::Path, root: &std::path::Path, cache: &std::path::Path) {
    let (ok, out) = run_with_stdlib(script, root, cache);
    assert!(ok && out.contains("v=1"), "first run: {out}");
    let src = std::fs::read_to_string(script).expect("script");
    std::fs::write(script, format!("{src}// edited\n")).expect("edit");
    for _ in 0..2 {
        let (ok, out) = run_with_stdlib(script, root, cache);
        assert!(ok && out.contains("v=1"), "warming run: {out}");
    }
}

/// A scratch `<root>/default/` holding the real stdlib plus one probe file.
fn scratch_stdlib(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("loft_b1_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dflt = root.join("default");
    std::fs::create_dir_all(&dflt).expect("scratch default/");
    for e in std::fs::read_dir(workspace_root().join("default")).expect("default/") {
        let p = e.expect("entry").path();
        if p.extension().and_then(|x| x.to_str()) == Some("loft") {
            std::fs::copy(&p, dflt.join(p.file_name().expect("name"))).expect("copy");
        }
    }
    std::fs::write(
        dflt.join("99_b1_probe.loft"),
        "pub fn b1probe() -> integer { 1 }\n",
    )
    .expect("probe");
    (root, dflt)
}

#[test]
fn an_edited_stdlib_function_is_never_served_from_a_cache() {
    let (root, dflt) = scratch_stdlib("edit");
    let cache = root.join("cache");
    let script = root.join("prog.loft");
    std::fs::write(&script, "fn main() { print(\"v={b1probe()}\\n\"); }\n").expect("script");
    warm_through_an_edit(&script, &root, &cache);
    std::fs::write(
        dflt.join("99_b1_probe.loft"),
        "pub fn b1probe() -> integer { 2 }\n",
    )
    .expect("edit");
    let (ok, out) = run_with_stdlib(&script, &root, &cache);
    assert!(
        ok && out.contains("v=2"),
        "an edited stdlib must be re-read, got: {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_added_stdlib_file_is_seen_after_an_edit_loop() {
    let (root, dflt) = scratch_stdlib("add");
    let cache = root.join("cache");
    let script = root.join("prog.loft");
    std::fs::write(&script, "fn main() { print(\"v={b1probe()}\\n\"); }\n").expect("script");
    warm_through_an_edit(&script, &root, &cache);
    std::fs::write(
        dflt.join("98_b1_extra.loft"),
        "pub fn b1extra() -> integer { 7 }\n",
    )
    .expect("add");
    // An edit: the program cache misses, so the stdlib must come from a parse or a cache
    // that knows about the new file.
    std::fs::write(&script, "fn main() { print(\"v={b1extra()}\\n\"); }\n").expect("script");
    let (ok, out) = run_with_stdlib(&script, &root, &cache);
    assert!(
        ok && out.contains("v=7"),
        "an added stdlib file must be seen, got: {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_removed_stdlib_file_is_never_served_from_a_cache() {
    let (root, dflt) = scratch_stdlib("remove");
    let cache = root.join("cache");
    let script = root.join("prog.loft");
    std::fs::write(&script, "fn main() { print(\"v={b1probe()}\\n\"); }\n").expect("script");
    warm_through_an_edit(&script, &root, &cache);
    std::fs::remove_file(dflt.join("99_b1_probe.loft")).expect("remove");
    let (ok, out) = run_with_stdlib(&script, &root, &cache);
    assert!(
        !ok && !out.contains("v=1"),
        "a removed stdlib function must be refused, not served from a cache: {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
