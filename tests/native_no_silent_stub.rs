// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#993 — no function in the generated Rust may be both CALLED and EMPTY.
//!
//! `output_function` escalates an unimplemented native to `compile_error!` when it is
//! reachable and `todo!()` when it is not — "fail at startup, not runtime". Both legs
//! gated that on `*def.returned() != Type::Void`, so a VOID unimplemented native was
//! emitted as `{}`: a function that compiles, is callable, and does nothing. The
//! principle had no effect on the half of the surface where the failure is silent rather
//! than a panic.
//!
//! That asymmetry is what hid the par discard route for its whole life — `--native`
//! emitted `n_parallel_discard`'s declaration with an empty body, and the only thing that
//! turned "runs no workers" into a visible failure was an unrelated arity mismatch
//! rustc refused (loft#987).
//!
//! The guard is gone from both legs. Nothing else had to change: the internal leg's
//! `reachable` already answers *"is this def actually called through its declaration"* —
//! it excludes a `#rust` body (inlined at the call site), an interface or T-param stub,
//! and a def with a registered `OpEmitter` (whose call sites are rewritten). Exactly one
//! built-in was relying on the silence, `yield_frame`, and its no-op is now WRITTEN as
//! `#rust "()"` rather than falling out of "nobody implemented it".
//!
//! The cell below is a property of the emitted source rather than a list of names, so it
//! keeps holding for built-ins that do not exist yet — which is the whole point of a
//! guard for a silence.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Reaches the routes that carry the void stubs: `par` with an empty body (discard), a
/// `par` whose body names its result (the queue family and its `buf_drop` siblings),
/// `eprint` (a `#rust` inline), and `yield_frame` (the deliberate native no-op).
const PROBE: &str = "fn sq993(n: integer) -> integer { n * n }\n\
fn main() {\n\
\x20 rows = [1, 2, 3];\n\
\x20 for a in rows par(b = sq993(a), 2) { }\n\
\x20 total = 0;\n\
\x20 for a in rows par(b = sq993(a), 2) { total += b; }\n\
\x20 eprint(\"e\");\n\
\x20 yield_frame();\n\
\x20 println(\"total {total}\");\n\
}\n";

fn emit(tag: &str) -> String {
    let src = std::env::temp_dir().join(format!("loft_993_{tag}_{}.loft", std::process::id()));
    let out_rs = std::env::temp_dir().join(format!("loft_993_{tag}_{}.rs", std::process::id()));
    fa::write(&src, PROBE).expect("write probe");
    let st = Command::new(loft_bin())
        .args(["--native-emit", out_rs.to_str().expect("path")])
        .arg(&src)
        .env("LOFT_TIMEOUT", "300")
        .status()
        .expect("spawn loft");
    assert!(st.success(), "--native-emit must succeed");
    let rs = fa::read_to_string(&out_rs).expect("read emitted Rust");
    let _ = fa::remove_file(&src);
    let _ = fa::remove_file(&out_rs);
    rs
}

/// Every generated `fn name(…) { }` whose body is empty, in source order.
fn empty_bodied(rs: &str) -> Vec<String> {
    let lines: Vec<&str> = rs.lines().collect();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if !l.starts_with("fn ") || !l.trim_end().ends_with('{') {
            continue;
        }
        if lines.get(i + 1).map(|n| n.trim()) != Some("}") {
            continue;
        }
        if let Some(name) = l["fn ".len()..].split('(').next() {
            out.push(name.to_string());
        }
    }
    out
}

/// The invariant, stated over the emitted source rather than over a list of names: a
/// function with an empty body may exist (an unreachable declaration is harmless), but
/// nothing may CALL it — a call that reaches an empty body is a silent no-op, which is
/// the shape loft#993 is about.
#[test]
fn no_generated_function_is_both_called_and_empty() {
    let rs = emit("prop");
    let mut silent: Vec<String> = Vec::new();
    for name in empty_bodied(&rs) {
        // A call is the name followed by `(` somewhere that is not its own `fn` line.
        let called = rs.lines().any(|l| {
            !l.starts_with(&format!("fn {name}("))
                && (l.contains(&format!("{name}(cell")) || l.contains(&format!(" {name}(")))
        });
        if called {
            silent.push(name);
        }
    }
    assert!(
        silent.is_empty(),
        "these generated functions are CALLED and do nothing — a void native with no \
         lowering, emitted as `{{}}` because the escalation used to skip void returns \
         (loft#993): {silent:?}"
    );
}

/// The control, and it changed shape while being written — which is the finding.
///
/// It first asserted the emit still CARRIES empty-bodied declarations, so that the
/// property above could not pass vacuously. It does not: lifting the guard took the count
/// to ZERO for this program. Every void stub is now either a loud `todo!()` or a no-op the
/// declaration writes down, and an emit with no empty bodies at all is the strongest
/// version of the invariant rather than a hole in it.
///
/// So the control pins the SIGNATURE of the change instead: a void built-in with no
/// lowering — `n_parallel_discard`, the one loft#987 was about — must appear as a loud
/// stub. That is what the guard used to skip, and a regression would put `{}` back.
#[test]
fn a_void_builtin_with_no_lowering_is_a_loud_stub() {
    let rs = emit("ctl");
    assert!(
        rs.contains("todo!(\"native function n_parallel_discard\")"),
        "`n_parallel_discard` returns void and has no body of its own — it must be emitted \
         LOUD, the way a value-returning one always was (loft#993)"
    );
    assert!(
        rs.contains("n_parallel_discard_native(cell,"),
        "…while its CALL goes to the runtime helper, which is what makes the loud \
         declaration unreachable rather than a refusal (loft#987)"
    );
    assert!(
        !rs.contains("compile_error!"),
        "nothing in this probe is a reachable unimplemented native, so the escalation \
         must stop at `todo!()` — a `compile_error!` here would mean the lifted guard \
         started refusing something that is lowered elsewhere"
    );
}

/// `yield_frame` keeps working on both backends — the no-op is deliberate, so making the
/// silence loud must not have turned it into a panic or a refusal.
#[test]
fn yield_frame_still_runs_on_both_backends() {
    for backend in ["--interpret", "--native"] {
        let src = std::env::temp_dir().join(format!(
            "loft_993_yf_{}_{}.loft",
            backend.trim_start_matches('-'),
            std::process::id()
        ));
        fa::write(
            &src,
            "fn main() { n = 0; while n < 3 { println(\"frame {n}\"); n += 1; yield_frame(); } }\n",
        )
        .expect("write probe");
        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(&src)
            .env("LOFT_TIMEOUT", "300")
            .output()
            .expect("spawn loft");
        let _ = fa::remove_file(&src);
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.status.success() && all.contains("frame 2"),
            "[{backend}] `yield_frame` is a deliberate no-op on a native binary — it has \
             no interpreter state to resume — and must stay one:\n{all}"
        );
    }
}
