// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! An entry function returning a heap value, and the ownership, capture and refusal regressions
//! that follow it.

use super::*;

// ── #618: an entry function returning a heap value ───────────────────────────
// `ref_return` promotion makes a returned local BE the caller's hidden return
// buffer, so the body writes straight into it.  An ordinary call site allocates
// that buffer before the call; `execute_argv` pushed a bare `DbRef::NULL`, so
// every element write addressed `stores[u16::MAX]` and aborted with
// "index out of bounds: the len is 2 but the index is 65535".
//
// `ReplSession::value_of` was the reported symptom, but the entry contract is
// what broke, so guard it directly here: a plain `fn main() -> vector<integer>`
// reproduced it identically under `--interpret`.  Struct returns always worked
// (their promoted body opens with its own `OpDatabase`) and are kept as the
// negative control — a "fix" that regressed them would pass the vector cases.
//
// Interpreter only: `--native` cannot yet compile ANY heap-returning entry fn
// (its generated `main` omits the hidden buffer argument → rustc E0428/E0061),
// a separate pre-existing gap on the same contract.
fn run_entry_returning(code: &str) -> (State, loft::data::Data) {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(code, "issue618", false);
    assert!(
        p.diagnostics.lines().is_empty(),
        "Parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute_argv("main", &p.data, &[]);
    (state, p.data)
}

#[test]
fn issue618_entry_fn_returns_bare_local_vector() {
    // Pre-fix: aborted here rather than returning.  The `println` makes the
    // built value observable even though the entry's return is discarded, so a
    // silently-empty buffer cannot pass as success.
    let (state, _) = run_entry_returning(
        "fn main() -> vector<integer> {\n  v = [1, 2, 3];\n  println(\"{v}\");\n  v\n}",
    );
    assert!(
        state.database.runtime_error.is_none(),
        "entry returning a bare local vector must not fault"
    );
}

#[test]
fn issue618_entry_fn_returns_vector_shapes() {
    // One axis per case: element kind, nesting depth, and zero cardinality —
    // every shape routes through the same hidden-buffer contract.
    for code in [
        "fn main() -> vector<text> {\n  v = [\"a\", \"b\"];\n  println(\"{v}\");\n  v\n}",
        "fn main() -> vector<vector<integer>> {\n  v = [[1, 2], [3]];\n  println(\"{v}\");\n  v\n}",
        "fn main() -> vector<integer> {\n  v: vector<integer> = [];\n  println(\"{v}\");\n  v\n}",
        // An element wider than 32 bits — the report's second signature.
        "fn main() -> vector<integer> {\n  v = [9000000000, 0];\n  println(\"{v}\");\n  v\n}",
    ] {
        let (state, _) = run_entry_returning(code);
        assert!(
            state.database.runtime_error.is_none(),
            "entry heap return faulted for: {code}"
        );
    }
}

#[test]
fn issue618_entry_fn_returning_struct_still_works() {
    // Negative control: this path allocates from the sentinel itself and was
    // never broken — it must stay that way.
    let (state, _) = run_entry_returning(
        "struct P { x: integer }\nfn main() -> P {\n  p = P { x: 1 };\n  println(\"{p}\");\n  p\n}",
    );
    assert!(
        state.database.runtime_error.is_none(),
        "entry returning a struct must keep working"
    );
}

// #654 — past ~32 KB of function body the interpreter's jump displacement was
// truncated to 16 bits, so a `while true` took its backward jump to an arbitrary
// address: the body ran ONCE, execution fell out of the loop, and main returned
// status 0 with no diagnostic.  A log ended mid-story with nothing wrong in it.
//
// The filed scope was the backward `while` jump, but a boundary matrix showed
// every jump class truncating — `while`, `for`, `break`, and the forward skips of
// `if` and `else` — because all of them encode a displacement the same way.  So
// the fix is at the encoding, not at any one construct: `OpGotoWord` /
// `OpGotoFalseWord` now carry a 32-bit displacement, which covers the whole
// `code_pos` space and so cannot move the threshold somewhere further out.
//
// `--native` was always correct here (it emits real Rust control flow and never
// reads these operands), which is why this guards the interpreter specifically.
#[test]
fn issue_654_jumps_survive_a_body_past_the_16_bit_displacement() {
    // ~2400 filler statements puts the body comfortably past 32 KB of emitted
    // bytecode; at 1484 the old encoder was still correct and at 1485 it was not,
    // so a guard at the boundary itself would be one statement from vacuous.
    //
    // The filler ACCUMULATES rather than assigning throwaway locals: it has to be
    // warning-free to compile here, and a running total lets each case assert how
    // many times the body actually ran.
    const N: i64 = 2400;
    let filler: String = (0..N).map(|i| format!("acc = acc + {i}; ")).collect();
    let once = N * (N - 1) / 2; // the filler's contribution per execution

    // One case per jump class.  Each asserts a VALUE, not merely that it ran:
    // the failure mode was silent fall-through, which a run-to-completion check
    // would have called a pass.
    let cases: [(&str, String); 5] = [
        // backward jump — `while`
        (
            "while",
            format!(
                "fn test() {{ acc = 0; i = 0; while true {{ i = i + 1; if i > 3 {{ break; }} {filler} }} \
             assert(i == 4, \"while ran {{i}} times, expected 4\"); \
             assert(acc == {}, \"while body ran the wrong number of times\"); }}",
                once * 3
            ),
        ),
        // backward jump — counted `for`
        (
            "for",
            format!(
                "fn test() {{ acc = 0; n = 0; for _ in 0..4 {{ n = n + 1; {filler} }} \
             assert(n == 4, \"for ran {{n}} times, expected 4\"); \
             assert(acc == {}, \"for body ran the wrong number of times\"); }}",
                once * 4
            ),
        ),
        // forward jump — `break` out of a huge body
        (
            "break",
            format!(
                "fn test() {{ acc = 0; i = 0; while true {{ i = i + 1; if i > 2 {{ break; }} {filler} }} \
             assert(i == 3, \"break left the loop at {{i}}, expected 3\"); \
             assert(acc == {}, \"break body ran the wrong number of times\"); }}",
                once * 2
            ),
        ),
        // forward jump — skipping a huge `if` body that must NOT run
        (
            "if",
            format!(
                "fn test() {{ acc = 0; x = 0; if x > 100 {{ {filler} }} \
             assert(acc == 0, \"the untaken if body ran\"); \
             assert(x == 0, \"execution did not reach the join\"); }}"
            ),
        ),
        // forward jump — skipping a huge `else` arm
        (
            "else",
            format!(
                "fn test() {{ acc = 0; taken = 0; if acc < 100 {{ taken = 1; }} else {{ {filler} taken = 2; }} \
             assert(taken == 1, \"took the wrong arm: {{taken}}\"); \
             assert(acc == 0, \"the untaken else arm ran\"); }}"
            ),
        ),
    ];

    for (label, src) in &cases {
        let (mut state, data) = compile_for_production(src);
        attach_production_logger(&mut state);
        state.execute("test", &data);
        assert!(
            !state.database.had_fatal,
            "#654: the `{label}` jump misbehaved past a 32 KB body"
        );
    }
}

// #655 — a `&boolean` parameter that is actually assigned panicked codegen with
// "Unknown referenced variable type: boolean".  Every other scalar reference type
// worked, which is what made it sting: `&boolean` is the natural shape for a
// two-state out-parameter, reached for right after `&integer` has just worked for
// the count beside it (moros hit it on `fn do_wall(open: &boolean, ax: &float, …)`).
//
// FOUR sites, not one.  The filed panic was the interpreter's READ path; fixing it
// exposed the interpreter WRITE path, and getting that far exposed two native ones.
// The root asymmetry is that a plain `boolean` parameter renders as Rust `bool`
// while a `&boolean` renders as `&mut u8`, because a boolean LOCAL is the tri-state
// storage byte (0/1/255, null-capable).  So the deref sites convert, symmetrically:
// read `*p == 1` (a null reads as false), write `u8::from(..)`.
//
// The matrix below is the reason all four were found — the filed scope was the
// parameter alone, and probes for a local `&`-link and for READING the flag in a
// condition each failed on native after the parameter case was green.
#[test]
fn issue_655_ampersand_boolean_reads_and_writes() {
    let cases: [(&str, &str); 7] = [
        // the filed reproducer
        (
            "negate",
            "fn flip(b: &boolean) { b = !b; } \
                    fn test() { x = false; flip(x); assert(x == true, \"negate\"); }",
        ),
        // a constant, not a negation — the write path without a read
        (
            "const",
            "fn setit(b: &boolean) { b = true; } \
                   fn test() { x = false; setit(x); assert(x == true, \"const\"); }",
        ),
        // the other direction, so a test that only ever produced `true` cannot pass
        (
            "from_true",
            "fn flip(b: &boolean) { b = !b; } \
                       fn test() { x = true; flip(x); assert(x == false, \"from true\"); }",
        ),
        // two flags at once — each must write its own slot
        (
            "two",
            "fn both(a: &boolean, b: &boolean) { a = true; b = false; } \
                 fn test() { p = false; q = true; both(p, q); \
                 assert(p == true, \"first\"); assert(q == false, \"second\"); }",
        ),
        // the shape moros actually wrote: a flag beside the scalars that worked
        (
            "mixed",
            "fn do_wall(open: &boolean, ax: &float, n: &integer) \
                   { open = true; ax = ax + 1.5; n = n + 1; } \
                   fn test() { o = false; x = 1.0; c = 0; do_wall(o, x, c); \
                   assert(o == true, \"flag\"); assert(x == 2.5, \"float\"); \
                   assert(c == 1, \"int\"); }",
        ),
        // a local `&`-link rather than a parameter — a separate native path
        (
            "local_link",
            "fn test() { a = false; b = &a; b = true; \
                        assert(a == true, \"local link\"); }",
        ),
        // READING the flag through the reference, which native compiled as `u8`
        // where a `bool` was required
        (
            "read",
            "fn setpair(b: &boolean, out: &integer) \
                  { if b { out = 1; } else { out = 2; } b = !b; } \
                  fn test() { x = true; n = 0; setpair(x, n); \
                  assert(x == false, \"flipped\"); assert(n == 1, \"read as true\"); }",
        ),
    ];
    for (label, src) in &cases {
        let (mut state, data) = compile_for_production(src);
        attach_production_logger(&mut state);
        state.execute("test", &data);
        assert!(
            !state.database.had_fatal,
            "#655: `&boolean` case `{label}` misbehaved"
        );
    }
}

// #656 — a library that qualifies a call with its OWN name (`dlib::shout(x)` inside
// `src/dlib.loft`) made the parser resolve that name back to the file it was already
// parsing and load it a second time.  Every definition re-registered, which surfaced
// as "cannot redefine method `shout` on `text`" pointing at the file's own line — an
// error about the source disagreeing with itself.
//
// A free function only duplicated silently (it showed up twice in `loft api-surface`
// output, the tell that went unread); a METHOD made it fatal.  Published `regex`
// carries exactly that shape, so `loft api-surface` — and with it `loft compat api` /
// `compat floor` / `compat check`, which all route through it — reported the library
// as unreadable while its own test suite passed.
#[test]
fn issue_656_self_qualified_reference_does_not_reparse_the_file() {
    let dir = std::env::temp_dir().join(format!("loft_656_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).unwrap();
    // A METHOD plus a self-qualified call — both halves are needed: the method is
    // what makes the re-registration fatal rather than merely duplicated.
    let file = dir.join("dlib.loft");
    fa::write(
        &file,
        "pub fn shout(self: text) -> text { return self; }\n\
         pub fn go(x: text) -> text { return dlib::shout(x); }\n",
    )
    .unwrap();

    let mut p = Parser::new();
    p.lib_dirs.push(dir.to_string_lossy().to_string());
    let _ = p.parse_dir(
        &format!("{}/default", env!("CARGO_MANIFEST_DIR")),
        true,
        false,
    );
    p.parse(&file.to_string_lossy(), false);
    let level = p.diagnostics.level();
    let report = format!("{}", p.diagnostics);
    let _ = fa::remove_dir_all(&dir);

    assert!(
        level < loft::diagnostics::Level::Error,
        "#656: a self-qualified reference must not re-parse its own file:\n{report}"
    );
}

// #656, second shape — a package that names itself while ANOTHER library is loaded.
//
// The two halves failed differently for the same reason.  With no other library in
// play the self-qualified name got loaded (and the file re-parsed) — the first case
// above.  Once a `use <dep>` has loaded something else, the main file's own name no
// longer resolves at all and the parser reported "Unknown library 'glib'" — naming
// the library it was reading at that moment.  `use_names` deliberately never holds
// the main file (source 1 is reserved so a user def can shadow a prelude name), so a
// package simply could not name itself.
//
// Published `graphics` is the real case: `graphics::color_r(..)` in a package that
// also does `use mesh3d; use glb;`.  It reproduced on 0.5.0, not just on main, and
// was the last package `loft compat floor` could not measure.  The dependency here
// is a local file rather than a registry package, so the test needs no network and
// no install.
#[test]
fn issue_656_package_can_name_itself_while_another_library_is_loaded() {
    let dir = std::env::temp_dir().join(format!("loft_656b_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(dir.join("src")).unwrap();
    // The other library — its presence is the whole point, so `use` must resolve.
    fa::write(
        dir.join("src").join("gdep.loft"),
        "pub fn helper(x: integer) -> integer { return x * 2; }\n",
    )
    .unwrap();
    fa::write(
        dir.join("loft.toml"),
        "[package]\nname = \"glib\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\
         [library]\nentry = \"src/glib.loft\"\n",
    )
    .unwrap();
    let file = dir.join("src").join("glib.loft");
    fa::write(
        &file,
        "use gdep;\n\
         pub fn base(x: integer) -> integer { return x + 1; }\n\
         pub fn far(x: integer) -> integer { return glib::base(x) + gdep::helper(x); }\n",
    )
    .unwrap();

    let mut p = Parser::new();
    p.lib_dirs
        .push(dir.join("src").to_string_lossy().to_string());
    let _ = p.parse_dir(
        &format!("{}/default", env!("CARGO_MANIFEST_DIR")),
        true,
        false,
    );
    p.parse(&file.to_string_lossy(), false);
    let level = p.diagnostics.level();
    let report = format!("{}", p.diagnostics);
    let _ = fa::remove_dir_all(&dir);

    assert!(
        level < loft::diagnostics::Level::Error,
        "#656: a package must be able to name itself with another library loaded:\n{report}"
    );
}

// Library compatibility contract, step 7 — the RELEASE gate proves the WHOLE claim.
//
// A PR pays O(1): latest + declared floor + one random interior release. That answers
// "did this change break something". Publishing asks a different question — "is
// everything this package promises actually true" — and it is the only one of the two
// that is a promise to people who cannot ask the package a question.
//
// The pressure this must resist is truncation. Verifying what fits in the time available
// and reporting green produces a release claiming a floor it never checked, which is
// worse than no check at all because it carries the authority of one. So an overrun is a
// FAILURE, and the remedy is named rather than implied: cost is proportional to the
// CLAIM, so narrowing the floor shrinks it.
//
// Driven through the real binary, because the property under test is an EXIT CODE — the
// thing the release script branches on. A unit test of the message would have passed
// while the gate returned 0. `LOFT_HOME` isolates the install cache so the window is
// built here rather than depending on whatever this machine happens to have installed.
#[test]
fn compat_full_window_budget_overrun_fails_the_release() {
    let loft = env!("CARGO_BIN_EXE_loft");
    let root = std::env::temp_dir().join(format!("loft_s7_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    let pkg = root.join("pkg");
    fa::create_dir_all(pkg.join("src")).unwrap();

    let manifest = |v: &str| {
        format!(
            "[package]\nname = \"s7pkg\"\nversion = \"{v}\"\nloft = \">=0.8\"\n\
             api_compatible_with  = \"0.1.0\"\ndata_compatible_with = \"0.1.0\"\n\
             [library]\nentry = \"src/s7pkg.loft\"\n"
        )
    };
    let source = "pub fn one(x: integer) -> integer { return x + 1; }\n";
    fa::write(pkg.join("loft.toml"), manifest("0.4.0")).unwrap();
    fa::write(pkg.join("src").join("s7pkg.loft"), source).unwrap();

    // Three earlier releases in an isolated install cache — a real window to walk, so
    // "budget exhausted" is reachable and "verified everything" means something.
    for v in ["0.1.0", "0.2.0", "0.3.0"] {
        let d = root.join(".loft/registry").join(format!("s7pkg-{v}"));
        fa::create_dir_all(d.join("src")).unwrap();
        fa::write(d.join("loft.toml"), manifest(v)).unwrap();
        fa::write(d.join("src").join("s7pkg.loft"), source).unwrap();
    }

    // The check asks the registry which releases exist (loft#2599): an index that lists
    // none of this package's keeps the run off the network and the window as built above.
    let index = root.join("index.json");
    fa::write(&index, r#"{"schema_version":1,"packages":{}}"#).unwrap();
    let index_url = format!("file://{}", index.display().to_string().replace('\\', "/"));

    let run = |budget: &str| {
        loft::platform::process::harness_command(loft)
            .args(["compat", "check", "--full", "--allow-unsigned"])
            .current_dir(&pkg)
            .env("LOFT_HOME", &root)
            .env("LOFT_REGISTRY_URL", &index_url)
            .env("LOFT_COMPAT_BUDGET", budget)
            .output()
            .expect("run loft compat check --full")
    };
    let text = |o: &std::process::Output| {
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    };

    // Generous budget: the whole window is walked and the claim holds.
    let ok = run("600");
    let ok_txt = text(&ok);
    // Zero budget: nothing can be proved, so it must not report success.
    let over = run("0");
    let over_txt = text(&over);
    let _ = fa::remove_dir_all(&root);

    assert!(
        ok.status.success(),
        "#step7: an unbroken window must pass:\n{ok_txt}"
    );
    assert!(
        ok_txt.contains("whole claim verified"),
        "#step7: a pass must say the WHOLE claim was proved:\n{ok_txt}"
    );
    assert!(
        ok_txt.contains("0.1.0") && ok_txt.contains("0.2.0") && ok_txt.contains("0.3.0"),
        "#step7: --full must report every release, not a sample:\n{ok_txt}"
    );

    assert!(
        !over.status.success(),
        "#step7: a budget overrun must FAIL the release, not pass:\n{over_txt}"
    );
    assert!(
        over_txt.contains("BUDGET EXCEEDED") && over_txt.contains("never checked"),
        "#step7: an overrun must quantify what went unverified:\n{over_txt}"
    );
    assert!(
        over_txt.contains("Narrow the claim"),
        "#step7: an overrun must name the remedy:\n{over_txt}"
    );
}

// loft#664 — element-slot ownership must be REPRESENTABLE when the container is a
// field DbRef.
//
// A vector-literal element never owns a store: its record is the slot the enclosing
// `OpNewRecord` carved out of the container.  That fact was encoded only as a
// DEPENDENCY on the container VARIABLE, so a vector living inside an enum payload —
// addressed by a field DbRef, with no variable to depend on — produced an EMPTY dep
// list, and empty reads as "owns its store".  The answer came back wrong rather than
// unknown, and every consumer of the predicate inherited it (loft#660 surfaced through
// `parse_object`, which allocated a fresh record over the slot: a silent corruption
// and a SIGSEGV at depth 2, patched there by matching the `_elm` NAME prefix).
//
// This asserts the FACT, not a value, because the value is already right — #660's name
// proxy covered the one consumer that acted on it, so a behaviour test would pass with
// or without the fix and prove nothing.  What must hold is that the element reports
// "does not own a store" WHILE its dep list is empty: that combination is exactly what
// only a marker at the mint site can express, and it is what retires the name proxy.
#[test]
fn issue_664_element_in_enum_payload_is_not_owning() {
    const SRC: &str = r#"
struct L664 { n: integer }
enum B664 { NilB, Items { items: vector<L664> } }
enum E664 { NilE, Val { v: B664 } }

fn build664() -> E664 {
  return Val { v: Items { items: [ L664 { n: 7 }, L664 { n: 9 } ] } };
}
"#;
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.parse_str(SRC, "<issue-664>", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "#664 source must parse clean: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);

    let d_nr = p.data.def_nr("n_build664");
    assert_ne!(d_nr, u32::MAX, "#664: build664 must be defined");
    let vars = p.data.def(d_nr).variables();

    let mut checked = 0;
    for v in 0..vars.var_count() {
        let v = v as u16;
        if !vars.name(v).starts_with("_elm") {
            continue;
        }
        checked += 1;
        // The dep list is the part that CANNOT carry the fact here: the container is a
        // field DbRef, so there is no variable to name.  Left as the only encoding, an
        // empty list is read as ownership.
        assert!(
            vars.tp(v).depend().is_empty(),
            "#664: this element's container is a field DbRef, so it has no container \
             variable to depend on — the probe is aimed at the wrong construction if \
             '{}' has deps {:?}",
            vars.name(v),
            vars.tp(v).depend()
        );
        // The fact is now STATED at the mint site rather than inferred, so it holds
        // whatever the container turns out to be.  Without the marker this element
        // reports non-owning only because the field-initialiser path happens to also
        // set `skip_free` — a coincidence of two conditions, not one fact, and the
        // reason #660 had to fall back on the `_elm` name.
        assert!(
            vars.is_inline_ref(v),
            "#664: element '{}' must be marked non-owning where it is MINTED, not \
             wherever a consumer can infer it",
            vars.name(v)
        );
        assert!(
            !vars.owns_store(v),
            "#664: element '{}' has an empty dep list AND must still report that it \
             does not own a store — that is the whole gap, and inferring ownership \
             from deps alone gets it wrong",
            vars.name(v)
        );
    }
    assert!(
        checked > 0,
        "#664: the probe found no element variable — it proves nothing"
    );
}

// loft#675 — a heap-returning function carries exactly one hidden return buffer, and a
// cross-package return still works end to end.
//
// @PLAN59 reserves the hidden `__retbuf` at SIGNATURE parse, gated on the declared return
// already being a heap type. For a struct owned by a dependency resolved LAZILY (a
// registry package pulled in through the pending-deps loop) that is not knowable on pass
// 1: `glb`'s `fn glb_pos_min(…) -> Vec3` reads `Unknown(659)` there, and the stub's def
// number is LOWER than the real struct's, so the definitions it needs do not exist yet.
// The reservation never fired, `ref_return` grew the attribute in pass 2 instead, and
// that cross-pass arity growth stopped the published `input` package from compiling once
// the H5 guard became always-on. `reserve_late_return_buffers` closes it between the
// passes, where every type IS resolved.
//
// BE HONEST ABOUT WHAT THIS TEST IS. It pins the invariant and the end-to-end behaviour
// on a `path =` dependency chain, and it is NOT a reproduction of the bug: a `path` dep
// is parsed eagerly, so its types resolve at signature time and the late reservation
// never has to fire here. I could not vendor the lazy-registry ordering into a fixture.
// What actually catches a recurrence is `assert_pass2_def_attr_stable` — always-on, in
// every build, and the thing that caught this one; the fix was verified by re-running the
// real `input 0.2.0` suite with that assert at FULL strictness.
#[test]
fn issue_675_cross_library_heap_return_reserves_its_buffer() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.lib_dirs.push("tests/fixtures/libs".to_string());
    p.parse("tests/multilib/r675_cross_lib_return.loft", false);
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(
        errors.is_empty(),
        "#675 fixture must parse clean: {errors:?}"
    );

    // `pick675` lives in `r675_use`; the program imports `r675_app`, which uses it for
    // itself and does not pass it on (@C98), so look it up wherever it is declared.
    let d_nr = (0..p.data.definitions())
        .find(|&d| p.data.def(d).name() == "n_pick675")
        .unwrap_or(u32::MAX);
    assert_ne!(d_nr, u32::MAX, "#675: pick675 must be defined");
    // A heap-returning function carries exactly one hidden heap buffer, and it was there
    // before pass 2 started — otherwise `ref_return` had to GROW the signature, and any
    // caller already lowered against the short arity passes one argument too few (#662).
    // The count also has to be exactly one: reserving a second buffer for a function that
    // already has one would break the ABI just as thoroughly.
    let attrs: Vec<(String, bool)> = p
        .data
        .def(d_nr)
        .attributes()
        .iter()
        .map(|a| (a.name.clone(), a.hidden))
        .collect();
    let hidden_heap = p
        .data
        .def(d_nr)
        .attributes()
        .iter()
        .filter(|a| {
            a.hidden
                && matches!(
                    a.typedef,
                    loft::data::Type::Reference(_, _)
                        | loft::data::Type::Vector(_, _)
                        | loft::data::Type::Enum(_, true, _)
                )
        })
        .count();
    assert_eq!(
        hidden_heap, 1,
        "#675: a function returning a struct from a DEPENDENCY must carry exactly one heap \
         return buffer — attributes: {attrs:?}"
    );

    // And it must still run: the buffer is an ABI change, so a wrong one shows up as a
    // caller/callee argument mismatch rather than a wrong number.
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let config = RuntimeLogConfig {
        log_path: std::path::PathBuf::from("/dev/null"),
        production: true,
        ..Default::default()
    };
    state.database.logger = Some(Arc::new(Mutex::new(Logger::new(config, None))));
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "#675: the cross-library heap return failed at runtime"
    );
}

/// loft#677 — a function that appends to a struct PARAMETER and returns it must keep that
/// parameter in its return deps, so callers see the result for what it is: a borrow of the
/// argument, not a store of its own.
///
/// The value-level guard lives in `tests/scripts/return-borrow-of-mutated-arg.loft`; this
/// one asserts the FACT, because losing the dep is only sometimes fatal.  With a single
/// call the over-free lands on a store nothing reads again and every value still checks
/// out — which is exactly why the four minimal cases on the issue all passed while the
/// consumer segfaulted.  A signature assertion fails the moment the dep goes missing.
///
/// The append count is the axis: `ref_return`'s ladder counted records allocated as
/// CHILDREN of `o` and read ≥2 as "reassigned local, do not NRVO-promote".  For a
/// parameter there is nothing to promote, so the skip only deleted the borrow.
#[test]
fn issue_677_returned_mutated_param_keeps_its_borrow() {
    let src_path = std::env::temp_dir().join("loft_i677_ret_borrow.loft");
    fa::write(
        &src_path,
        "struct I677 { x: float }\n\
         struct S677 { name: text, tags: vector<integer>, items: vector<I677> }\n\
         fn one677(o: S677, t: integer) -> S677 { o.tags += [t]; o }\n\
         fn two677(o: S677, t: integer, it: I677) -> S677 { o.tags += [t]; o.items += [it]; o }\n\
         fn same677(o: S677, t: integer) -> S677 { o.tags += [t]; o.tags += [t]; o }\n\
         fn fresh677(o: S677) -> S677 { S677 { name: o.name, tags: [], items: [] } }\n\
         fn main() { s = S677 { name: \"c\", tags: [], items: [] }; two677(s, 1, I677 { x: 1.0 }); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "#677 fixture must parse clean: {:?}",
        p.diagnostics.lines()
    );

    // `o` is attribute 0 in each of these, so the return dep must name index 0.
    for (name, appends) in [("n_one677", 1), ("n_two677", 2), ("n_same677", 2)] {
        let d_nr = p.data.def_nr(name);
        assert_ne!(d_nr, u32::MAX, "#677: {name} must be defined");
        let deps = p.data.def(d_nr).returned().depend().clone();
        assert!(
            deps.contains(&0),
            "#677: {name} returns its own parameter `o` after {appends} append(s), so the \
             return type must borrow attr 0 — got deps {deps:?}.  An empty dep list tells \
             every caller the result is an owned store, and the caller then frees the store \
             it passed in."
        );
    }

    // The other side of the boundary: a function that really does build a FRESH record
    // must NOT claim to borrow its argument, or callers leak it instead.
    let fresh = p.data.def_nr("n_fresh677");
    assert_ne!(fresh, u32::MAX, "#677: fresh677 must be defined");
    assert!(
        p.data.def(fresh).returned().depend().is_empty(),
        "#677: fresh677 allocates its own record, so its return must stay dep-free — got {:?}",
        p.data.def(fresh).returned().depend()
    );
}

/// loft#682 — a closure record must free only the captures it ADOPTED.
///
/// Each reference / collection capture is stored as a 12-byte `DbRef`, and
/// `free_named`'s cascade used to free every one of them when the record died.
/// That is right for a store the defining frame owned and handed over (the frame's
/// scope-exit free is suppressed for it, which is what lets an escaping factory
/// closure outlive its frame, #323), and wrong for a captured PARAMETER, whose
/// caller owns the store and outlives the frame.  The consumer lost a whole
/// `hex_world::World` to it.
///
/// The value-level guard is `tests/scripts/682-closure-capture-borrow.loft`; this
/// one asserts the FACT, because an over-free is only sometimes fatal.  With one
/// call the freed store is never re-read and every value still checks out — which
/// is why the report arrived as a panic thousands of ops later in an unrelated
/// function, and why a value test alone can pass on slot-reuse luck.
///
/// Two directions matter, so both are asserted: a parameter / projection capture
/// must be BORROWED (over-free), and an owned local must stay ADOPTED (marking it
/// borrowed leaks instead).  The verdict is only knowable after `scopes::check` —
/// `p682_proj`'s `ch` parses as "borrows `w`" and a call-result capture parses as
/// a borrow that scope analysis later rewrites to owned — so the test runs the
/// check first, exactly as compilation does.
#[test]
fn issue_682_closure_capture_ownership_marker() {
    let src_path = std::env::temp_dir().join("loft_i682_capture_marker.loft");
    fa::write(
        &src_path,
        "struct P682C { v: float }\n\
         struct P682W { cells: vector<P682C>, tick: integer }\n\
         fn p682_param(w: P682W, x: float) -> float { f = fn(s: float) -> float { w.tick as float * s }; x }\n\
         fn p682_proj(w: P682W, x: float) -> float { c = w.cells[0]; f = fn(s: float) -> float { c.v * s }; x }\n\
         fn p682_owned(x: float) -> float { o = P682W { cells: [], tick: 3 }; f = fn(s: float) -> float { o.tick as float * s }; f(x) }\n\
         fn p682_cell(x: integer) -> integer { acc = 0; b = fn(n: integer) { acc = acc + n; }; b(x); acc }\n\
         fn main() { w = P682W { cells: [P682C { v: 1.0 }], tick: 7 };\n\
                     p682_param(w, 1.0); p682_proj(w, 1.0); p682_owned(1.0); p682_cell(2); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "#682 fixture must parse clean: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);

    // The lambdas are numbered in source order, so each closure record pairs with
    // the function above that defines it.
    let marker = |record: &str, capture: &str| -> bool {
        let d_nr = p.data.def_nr(record);
        assert_ne!(d_nr, u32::MAX, "#682: {record} must exist");
        let a = p.data.attr(d_nr, capture);
        assert_ne!(
            a,
            usize::MAX,
            "#682: {record} must carry the `{capture}` capture"
        );
        match p.data.attr_type(d_nr, a) {
            loft::data::Type::Reference(_, deps) => {
                assert!(
                    !deps.is_empty(),
                    "#682: {record}.{capture} must keep a share marker (a 12-byte DbRef); \
                     empty deps would store inline bytes and the closure would read a \
                     stale snapshot"
                );
                deps.is_borrowed_share()
            }
            other => panic!("#682: {record}.{capture} should be a Reference, got {other:?}"),
        }
    };

    assert!(
        marker("__closure_p682_param_0", "w"),
        "#682: a captured PARAMETER must be marked BORROWED — its caller owns the store \
         and outlives this frame, so the record's cascade freeing it destroys the \
         caller's value (that is the whole bug)"
    );
    assert!(
        marker("__closure_p682_proj_0", "c"),
        "#682: a PROJECTION local (`c = w.cells[0]`) views into someone else's store, so \
         it must be marked BORROWED too — a parameter-only fix leaves this half broken"
    );
    assert!(
        !marker("__closure_p682_owned_0", "o"),
        "#682: a local the frame OWNS must stay ADOPTED — `get_free_vars` suppresses its \
         scope-exit free, so the record's cascade is the only free there is and marking \
         it borrowed leaks the store instead"
    );
    assert!(
        !marker("__closure_p682_cell_0", "acc"),
        "#682: a mutated scalar boxed into a `__cell_<T>` is minted FOR this closure, so \
         the record owns it however the original binding was reached (plan-22 / C74)"
    );
}

/// loft#685 — a MUTATED scalar capture whose source is a PARAMETER.
///
/// The closure record stored the capture as a 12-byte cell `DbRef`
/// (`box_captured_names_for_outer_scalars`) while `flip_scalars_to_box_types`
/// skipped arguments, so the parameter stayed an 8-byte stack scalar and
/// `emit_lambda_code`'s `OpSetDbRef` read 12 bytes out of an 8-byte slot — taking
/// the fn-ref being built beside it with it.
///
/// Values live in `tests/scripts/685-mutated-scalar-param-capture.loft`; this
/// asserts the FACT the fix rests on, because the value test cannot distinguish
/// "the two halves agree" from "they disagree but the garbage happened to work".
/// The invariant: for a mutated capture, the closure record's field type and the
/// binding the enclosing frame holds under that name must be the SAME cell.
#[test]
fn issue_685_mutated_scalar_param_is_boxed_like_a_local() {
    let src_path = std::env::temp_dir().join("loft_i685_param_box.loft");
    fa::write(
        &src_path,
        "fn p685_arg(n: integer) -> integer { b = fn(k: integer) { n = n + k; }; b(1); n }\n\
         fn p685_loc(s: integer) -> integer { n = s; b = fn(k: integer) { n = n + k; }; b(1); n }\n\
         fn main() { p685_arg(1); p685_loc(1); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "#685 fixture must parse clean: {:?}",
        p.diagnostics.lines()
    );

    // Both functions must lower to the SAME shape — the argument case is the local
    // case, which is the whole point of the promotion.
    for (fname, record) in [
        ("n_p685_arg", "__closure_p685_arg_0"),
        ("n_p685_loc", "__closure_p685_loc_0"),
    ] {
        let rec = p.data.def_nr(record);
        assert_ne!(rec, u32::MAX, "#685: {record} must exist");
        let a = p.data.attr(rec, "n");
        assert_ne!(a, usize::MAX, "#685: {record} must carry the `n` capture");
        let cell = match p.data.attr_type(rec, a) {
            loft::data::Type::Reference(d, _) => d,
            other => panic!("#685: {record}.n must be a cell Reference, got {other:?}"),
        };
        assert!(
            p.data.def(cell).name().starts_with("__cell_"),
            "#685: {record}.n must point at a `__cell_<T>`, got `{}`",
            p.data.def(cell).name()
        );

        // The frame's binding for `n` must be that same cell.  A binding that is still
        // a bare scalar here is the bug: the record would hold a 12-byte DbRef field
        // fed from an 8-byte slot.
        let d_nr = p.data.def_nr(fname);
        assert_ne!(d_nr, u32::MAX, "#685: {fname} must be defined");
        let vars = &p.data.def(d_nr).variables;
        let v = vars.var("n");
        assert_ne!(v, u16::MAX, "#685: {fname} must bind the name `n`");
        assert!(
            matches!(vars.tp(v), loft::data::Type::Reference(d, _)
                if p.data.def(*d).name().starts_with("__cell_")),
            "#685: `n` in {fname} must be the boxed cell the record points at, got {:?} \
             — a bare scalar here means the closure record is fed 12 bytes from an \
             8-byte slot",
            vars.tp(v)
        );
        assert!(
            !vars.is_argument(v),
            "#685: `n` in {fname} must resolve to a shadow LOCAL, not the argument — \
             flipping the argument itself would change the call ABI"
        );
    }

    // The by-value contract: the argument is still an argument, untouched, so the
    // caller's value cannot be reached through the cell.
    let d_nr = p.data.def_nr("n_p685_arg");
    let vars = &p.data.def(d_nr).variables;
    let args = vars.arguments();
    assert_eq!(
        args.len(),
        1,
        "#685: promoting must not change the arity of `p685_arg` (the H5 two-pass \
         contract catches this too) — got {args:?}"
    );
    assert!(
        matches!(vars.tp(args[0]), loft::data::Type::Integer(_)),
        "#685: the parameter slot must stay a plain scalar, got {:?}",
        vars.tp(args[0])
    );
}

/// loft#687 — a mutated TEXT capture's STORAGE is decided per BINDING, not per function.
///
/// #685 could not serve a mutated `text` PARAMETER in a text-returning function and
/// refused it by name; plan-22 02d-vii had skipped text boxing whenever the parent
/// returned text.  That condition was a proxy for one real case — a text local that is
/// the function's RETURN SOURCE, which the return machinery already gives its own hidden
/// `&text` out-parameter — and as a proxy it was too wide (it also skipped a text local
/// the function does not return) and useless for a parameter, which has no indirection to
/// reuse.  Both halves now ask the binding: `RefVar` means "already has one".
///
/// The fixture puts BOTH bindings in ONE text-returning function, which is what no
/// per-function condition can get right.  Since loft#2297 both take a shared cell — `keep`
/// (returned) no longer becomes the hidden `&text` return buffer, so it has one indirection.  Values live in
/// `tests/scripts/687-mutated-text-param-capture.loft`; this asserts the storage, because
/// mixing the two representations for one binding is what used to segfault, and a value
/// test cannot see which one a green run picked.
#[test]
fn issue_687_mutated_text_capture_storage_is_per_binding() {
    let src_path = std::env::temp_dir().join("loft_i687_text_storage.loft");
    fa::write(
        &src_path,
        "fn ret687(seed: text) -> text { keep = seed; b = fn(k: text) { keep = keep + k; }; b(\"x\"); keep }\n\
         fn side687(seed: text) -> text { side = seed; b = fn(k: text) { side = side + k; }; b(\"x\"); \"p\" + side }\n\
         fn par687(n: text) -> text { b = fn(k: text) { n = n + k; }; b(\"x\"); n }\n\
         fn main() { ret687(\"a\"); side687(\"a\"); par687(\"a\"); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "#687: a mutated text parameter in a text-returning fn must COMPILE now: {:?}",
        p.diagnostics.lines()
    );

    let is_cell = |record: &str, capture: &str| -> bool {
        let rec = p.data.def_nr(record);
        assert_ne!(rec, u32::MAX, "#687: {record} must exist");
        let a = p.data.attr(rec, capture);
        assert_ne!(a, usize::MAX, "#687: {record} must carry `{capture}`");
        matches!(p.data.attr_type(rec, a),
            loft::data::Type::Reference(d, _) if p.data.def(d).name().starts_with("__cell_"))
    };

    // All three functions return text, so the retired per-function proxy gave all three
    // the same answer.  Each binding needs a different one.

    // `keep` IS the returned text.  Two indirections for one binding is the crash: a cell in
    // the record while the binding stays a `&text` stack pointer.  loft#2297 keeps a text
    // local the closure WRITES out of the hidden `&text` return buffer (inline, the closure
    // wrote its own copy and the write was lost), so it takes the cell — and the function
    // must then carry no hidden `&text` attribute for it.
    assert!(
        is_cell("__closure_ret687_0", "keep"),
        "#687 / loft#2297: a returned text local a closure writes takes a shared cell"
    );
    let ret = p.data.def_nr("n_ret687");
    assert!(
        !p.data
            .def(ret)
            .attributes()
            .iter()
            .any(|a| a.hidden && a.name == "keep"),
        "#687: a boxed `keep` must not ALSO be the hidden `&text` return buffer"
    );
    // `side` is not returned, so nothing else claims it and the shared cell is right.
    // The retired proxy skipped this one purely as collateral — the "too wide" half.
    assert!(
        is_cell("__closure_side687_0", "side"),
        "#687: a mutated text local the function does NOT return must take a shared cell"
    );
    // A parameter has no indirection of its own to reuse, so #685's shadow local takes
    // the cell like every other type.  This is the combination that used to be refused.
    assert!(
        is_cell("__closure_par687_0", "n"),
        "#687: a mutated text PARAMETER's shadow local must take a shared cell"
    );
}

/// loft#685 boundary — a value-const scalar parameter mutated through a closure stays
/// an error.
///
/// The closure-side write never reaches `validate_write`'s const guard (inside the
/// lambda the name is a capture, not a binding carrying the flag), so the check lives
/// at the promotion site.  Without it the fix would have quietly handed the closure a
/// writable cell for a read-only parameter — turning a crash into a silently accepted
/// contract violation, which is worse.
#[test]
fn issue_685_const_scalar_param_mutated_by_closure_is_rejected() {
    let src_path = std::env::temp_dir().join("loft_i685_const_param.loft");
    fa::write(
        &src_path,
        "fn c685(n: const integer) -> integer { b = fn(k: integer) { n = n + k; }; b(1); n }\n\
         fn main() { c685(1); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    let lines = p.diagnostics.lines().join("\n");
    assert!(
        p.diagnostics.level() >= loft::diagnostics::Level::Error,
        "#685: mutating a const parameter from a closure must stay an error.  \
         Diagnostics: {lines}"
    );
    assert!(
        lines.contains("const parameter") && lines.contains("from a closure"),
        "#685: the const refusal must say which binding and why, got: {lines}"
    );
}

/// loft#686 — a capture whose type comes from a struct declared LATER in the file.
///
/// Two faults composed. `Unknown(0)` is the "no type known" sentinel and names no
/// definition, but `copy_unknown_fields` read the `0` as a def number and gave the
/// field whatever definition #0 returns — `text` — so the closure body type-checked
/// against a type nothing in the program mentions. With that invented type gone the
/// field became unsized, and `fill_database` registers a struct while SKIPPING an
/// attribute it cannot size, so `finish` sized the record with the field left at
/// `position == u16::MAX` and the closure read its capture at offset 65535.
///
/// Values live in `tests/scripts/686-forward-declared-capture.loft`; these assert the
/// two FACTS, because the second fault was INTERMITTENT — a positionless field only
/// crashes when the bytes at offset 65535 happen to be fatal, so a green value run
/// proves much less than it looks.
#[test]
fn issue_686_forward_declared_capture_is_typed_and_positioned() {
    let src_path = std::env::temp_dir().join("loft_i686_forward_capture.loft");
    fa::write(
        &src_path,
        "fn q686(w: W686) -> float { ch = w.inner; f = fn(x: float) -> float { ch.q * x }; f(2.0) }\n\
         struct I686 { q: float }\n\
         struct W686 { inner: I686, tick: integer }\n\
         fn main() { w = W686 { inner: I686 { q: 5.0 }, tick: 7 }; q686(w); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "#686 fixture must parse clean: {:?}",
        p.diagnostics.lines()
    );

    let rec = p.data.def_nr("__closure_q686_0");
    assert_ne!(rec, u32::MAX, "#686: the closure record must exist");
    let a = p.data.attr(rec, "ch");
    assert_ne!(
        a,
        usize::MAX,
        "#686: the record must carry the `ch` capture"
    );

    // FACT 1 — the capture is typed as what it is, not as definition #0's return type.
    match p.data.attr_type(rec, a) {
        loft::data::Type::Reference(d, _) => assert_eq!(
            p.data.def(d).name(),
            "I686",
            "#686: the capture must resolve to the forward-declared struct, got `{}`",
            p.data.def(d).name()
        ),
        other => panic!(
            "#686: a struct capture must be a Reference; got {other:?}.  `Text` here is \
             the original bug — `Unknown(0)` resolved against definition #0"
        ),
    }

    // FACT 2 — the record's field is POSITIONED.  Skipping an unsized attribute while
    // still registering the struct left this at u16::MAX forever, and `finish_type`
    // will not revisit a sized type, so nothing downstream could repair it.
    let known = p.data.def(rec).known_type();
    assert_ne!(
        known,
        u16::MAX,
        "#686: the closure record must be laid out — a record left unregistered makes \
         `OpDatabase` allocate type u16::MAX"
    );
    let pos = p.database.position(known, "ch");
    assert_ne!(
        pos,
        u16::MAX,
        "#686: the `ch` field must have a real byte position; u16::MAX means the record \
         was sized while the field was still unresolved, and the closure then reads and \
         writes at offset 65535 (an INTERMITTENT crash, which is why this asserts the \
         position rather than trusting a green run)"
    );
}

/// loft#686 sibling — `Unknown(0)` must never be resolved as a reference to definition
/// #0.
///
/// It is the codebase-wide "no type known" sentinel (`Type::Unknown(0)` is what every
/// unresolved expression carries), so reading the `0` as a def number invents a type
/// from whatever happens to be defined first.  That is a lying fact rather than a
/// missing one: the field looked resolved, so nothing downstream questioned it, and the
/// error surfaced as `Unknown field text.cells` on a program with no `text` in sight.
#[test]
fn issue_686_nameless_unknown_is_not_resolved_against_def_zero() {
    let src_path = std::env::temp_dir().join("loft_i686_sentinel.loft");
    fa::write(
        &src_path,
        "fn s686(w: S686W) -> float { p = w.inner; f = fn(x: float) -> float { p.q * x }; f(1.0) }\n\
         struct S686I { q: float }\n\
         struct S686W { inner: S686I }\n\
         fn main() { w = S686W { inner: S686I { q: 2.0 } }; s686(w); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);

    // Definition #0's return type is what the old resolution handed the field.  Whatever
    // it is, no capture may end up with it by accident.
    let def_zero_ret = p.data.def(0).returned().clone();
    let rec = p.data.def_nr("__closure_s686_0");
    assert_ne!(rec, u32::MAX, "#686: the closure record must exist");
    let a = p.data.attr(rec, "p");
    assert_ne!(a, usize::MAX, "#686: the record must carry the `p` capture");
    let got = p.data.attr_type(rec, a);
    assert!(
        !matches!(&got, t if *t == def_zero_ret),
        "#686: the capture took definition #0's return type ({def_zero_ret:?}) — \
         `Unknown(0)` was resolved as a def reference again"
    );
    assert!(
        matches!(&got, loft::data::Type::Reference(d, _) if p.data.def(*d).name() == "S686I"),
        "#686: the capture must be the struct it projects out of, got {got:?}"
    );
}

/// loft#683 — an index key whose type comes from a definition declared LOWER in the
/// file must not be rejected.  The check ran in pass 1, which sees only what is
/// declared above the current point, so `h[keys_declared_below()]` failed while the
/// same code with the callee moved up compiled.  Pass 2 knows every signature, but it
/// only runs when pass 1 is error-free — so the premature error aborted the parse.
///
/// The values live in `tests/scripts/683-declaration-order-index-key.loft`.  This is
/// the must-fail half: deferring the check must not DELETE it.  A genuinely wrong key
/// type has to stay an error in BOTH declaration orders — a fix that simply stopped
/// reporting would pass the value test and fail here.
#[test]
fn issue_683_wrong_index_key_is_still_rejected_in_both_orders() {
    // (label, source) — identical programs, differing only in where `t683` sits.
    let cases = [
        (
            "callee above",
            "struct K683 { k: integer, v: integer }\n\
             fn t683() -> text { \"z\" }\n\
             fn u683(h: hash<K683[k]>) -> integer { r = h[t683()]; if r != null { return r.v; } 0 }\n\
             fn main() { h: hash<K683[k]> = []; u683(h); }\n",
        ),
        (
            "callee below",
            "struct K683 { k: integer, v: integer }\n\
             fn u683(h: hash<K683[k]>) -> integer { r = h[t683()]; if r != null { return r.v; } 0 }\n\
             fn t683() -> text { \"z\" }\n\
             fn main() { h: hash<K683[k]> = []; u683(h); }\n",
        ),
    ];
    for (label, src) in cases {
        let src_path = std::env::temp_dir().join("loft_i683_wrong_key.loft");
        fa::write(&src_path, src).unwrap();
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse(src_path.to_str().unwrap(), false);
        let lines = p.diagnostics.lines().join("\n");
        assert!(
            p.diagnostics.level() >= loft::diagnostics::Level::Error,
            "#683 ({label}): a text key on an integer-keyed hash must stay an error.  \
             Diagnostics: {lines}"
        );
        assert!(
            lines.contains("Invalid index key"),
            "#683 ({label}): the rejection must still be the index-key diagnostic, got: {lines}"
        );
        let _ = fa::remove_file(&src_path);
    }
}

/// loft#689 — a range asks a collection to walk its keys IN ORDER, so only an ordered
/// collection can answer it.  `hash` is unordered and `spatial` is Morton-ordered (its
/// range form is the coordinate slice `s[(x1,y1)..(x2,y2)]`), and neither could step a
/// scalar range: both walked off the end of their iterator and SIGSEGV'd.  An open
/// `coll[..]` crashed too — it is the same walk without bounds.
///
/// These are now refused at parse time.  The values for the collections that CAN answer
/// a range live in `tests/scripts/689-keyed-range-slice.loft`; this pins the refusal, and
/// the second half pins what must keep working — a fix that simply rejected every index
/// on a `hash` would pass the first half alone.
#[test]
fn issue_689_range_is_refused_on_an_unordered_collection() {
    let refused = [
        (
            "hash, bounded range",
            "struct H689 { k: integer, v: integer }\n\
             fn main() { h: hash<H689[k]> = []; n = 0; for r in h[1..3] { n += r.v; } }\n",
            "unordered",
        ),
        (
            "hash, open range",
            "struct H689 { k: integer, v: integer }\n\
             fn main() { h: hash<H689[k]> = []; n = 0; for r in h[..] { n += r.v; } }\n",
            "unordered",
        ),
        (
            "spatial, scalar range",
            "struct S689 { x: integer, y: integer, v: integer }\n\
             fn main() { s: spatial<S689[x,y]> = []; n = 0; for r in s[1..3] { n += r.v; } }\n",
            "COORDINATE slice",
        ),
    ];
    for (label, src, needle) in refused {
        let src_path = std::env::temp_dir().join("loft_i689_refused.loft");
        fa::write(&src_path, src).unwrap();
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse(src_path.to_str().unwrap(), false);
        let lines = p.diagnostics.lines().join("\n");
        assert!(
            p.diagnostics.level() >= loft::diagnostics::Level::Error,
            "#689 ({label}): a range on an unordered collection must be refused, not run.  \
             Diagnostics: {lines}"
        );
        assert!(
            lines.contains(needle),
            "#689 ({label}): the refusal must say WHY and what to use instead \
             (expected {needle:?}), got: {lines}"
        );
        let _ = fa::remove_file(&src_path);
    }

    // The forms a hash / spatial collection CAN answer must still compile.
    let allowed = [
        (
            "hash single-key lookup",
            "struct H689 { k: integer, v: integer }\n\
             fn main() { h: hash<H689[k]> = []; r = h[1]; if r != null { } }\n",
        ),
        (
            "hash whole-collection iteration",
            "struct H689 { k: integer, v: integer }\n\
             fn main() { h: hash<H689[k]> = []; n = 0; for r in h { n += r.v; } }\n",
        ),
        (
            "spatial coordinate slice",
            "struct S689 { x: integer, y: integer, v: integer }\n\
             fn main() { s: spatial<S689[x,y]> = []; n = 0; \
             for r in s[(0,0)..(5,5)] { n += r.v; } }\n",
        ),
    ];
    for (label, src) in allowed {
        let src_path = std::env::temp_dir().join("loft_i689_allowed.loft");
        fa::write(&src_path, src).unwrap();
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse(src_path.to_str().unwrap(), false);
        let lines = p.diagnostics.lines().join("\n");
        assert!(
            p.diagnostics.level() < loft::diagnostics::Level::Error,
            "#689 ({label}): this form is what the refusal tells users to write — \
             it must still compile.  Diagnostics: {lines}"
        );
        let _ = fa::remove_file(&src_path);
    }
}

/// loft#690 — a `for` loop whose variable name is already bound to a DIFFERENT type
/// must not silently reuse the earlier binding.
///
/// The reuse check existed but compared with `Type::is_same`, which answers "same KIND
/// of type": it reports any two `Reference`s as the same whatever struct they name.  So
/// `for r in as_a { … }  for r in as_b { … }` slipped through, `add_variable` handed the
/// second loop the FIRST binding — old var, old type, old dep — and the body read B's
/// records through A's layout.  No diagnostic and no crash, just wrong numbers.
///
/// loft#915 removed the inheritance instead of reporting it: each loop binds its OWN
/// variable, so there is no earlier binding for the second loop to take, and these
/// programs now COMPILE.  What is asserted is therefore the stronger property the
/// diagnostic only approximated — the second loop's variable really carries the SECOND
/// element type.  Each body below reads a field that exists only on its own struct, so
/// a leaked binding is an "unknown field" error rather than a silent wrong number, which
/// is what made a value test unconvincing here (two structs with identical fields
/// returned the right answer while still being undefined).  The value half, on both
/// backends, is `tests/scripts/915-loop-variable-per-loop.loft` cell c6.
#[test]
fn issue_690_loop_variable_binds_its_own_type() {
    let per_loop_binding = [
        (
            "two struct types",
            "struct A690 { k: integer, av: integer }\n\
             struct B690 { k: text, bv: integer }\n\
             fn main() { a: vector<A690> = [A690{k:1,av:10}]; b: vector<B690> = [B690{k:\"x\",bv:1}];\n\
             n = 0; for r in a { n += r.av; } for r in b { n += r.bv; } println(\"{n}\"); }\n",
        ),
        (
            "two structs with IDENTICAL layouts — still different types",
            "struct A690 { k: integer, av: integer }\n\
             struct C690 { k: integer, cv: integer }\n\
             fn main() { a: vector<A690> = [A690{k:1,av:10}]; c: vector<C690> = [C690{k:2,cv:20}];\n\
             n = 0; for r in a { n += r.av; } for r in c { n += r.cv; } println(\"{n}\"); }\n",
        ),
        (
            "two enum types",
            "enum E690 { Red, Green }\n\
             enum F690 { Up, Down }\n\
             fn main() { a: vector<E690> = [E690.Red]; b: vector<F690> = [F690.Up];\n\
             n = 0; for r in a { if r == E690.Red { n += 1; } } \
             for r in b { if r == F690.Up { n += 1; } } println(\"{n}\"); }\n",
        ),
        (
            "nested vectors of different element structs",
            "struct A690 { av: integer }\n\
             struct B690 { bv: integer }\n\
             fn main() { a: vector<vector<A690>> = [[A690{av:1}]]; \
             b: vector<vector<B690>> = [[B690{bv:2}]];\n\
             n = 0; for r in a { for e in r { n += e.av; } } \
             for r in b { for e in r { n += e.bv; } } println(\"{n}\"); }\n",
        ),
    ];
    for (label, src) in per_loop_binding {
        let src_path = std::env::temp_dir().join("loft_i690_per_loop.loft");
        fa::write(&src_path, src).unwrap();
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse(src_path.to_str().unwrap(), false);
        let lines = p.diagnostics.lines().join("\n");
        assert!(
            p.diagnostics.level() < loft::diagnostics::Level::Error,
            "#690 ({label}): each loop binds its own variable, so the second loop's field \
             read resolves against the SECOND type and this must compile.  Diagnostics: \
             {lines}"
        );
        let _ = fa::remove_file(&src_path);
    }

    // The tolerances are load-bearing: loft has FLAT variable scoping, so reusing one
    // loop name at the SAME type is idiomatic and must keep compiling.  Tightening the
    // comparison to `is_equal` must not reach any of these.
    let accepted = [
        (
            "same struct, two loops",
            "struct A690 { v: integer }\n\
             fn main() { a: vector<A690> = [A690{v:1}]; b: vector<A690> = [A690{v:2}];\n\
             n = 0; for r in a { n += r.v; } for r in b { n += r.v; } }\n",
        ),
        (
            "same struct through DIFFERENT collection kinds",
            "struct A690 { k: integer, v: integer }\n\
             fn main() { a: vector<A690> = [A690{k:1,v:10}]; s: sorted<A690[k]> = []; \
             s += A690{k:2,v:20};\n\
             n = 0; for r in a { n += r.v; } for r in s { n += r.v; } }\n",
        ),
        (
            "integers in both loops (differing ranges stay the same type)",
            "fn main() { a: vector<integer> = [1,2]; b: vector<integer> = [3];\n\
             n = 0; for r in a { n += r; } for r in b { n += r; } }\n",
        ),
        (
            "text in both loops (differing deps stay the same type)",
            "fn main() { a: vector<text> = [\"xy\"]; b: vector<text> = [\"z\"];\n\
             n = 0; for r in a { n += len(r); } for r in b { n += len(r); } }\n",
        ),
        (
            "`_` stays exempt across different structs",
            "struct A690 { v: integer }\n\
             struct B690 { v: integer }\n\
             fn main() { a: vector<A690> = [A690{v:1}]; b: vector<B690> = [B690{v:2}];\n\
             n = 0; for _ in a { n += 1; } for _ in b { n += 1; } }\n",
        ),
    ];
    for (label, src) in accepted {
        let src_path = std::env::temp_dir().join("loft_i690_accepted.loft");
        fa::write(&src_path, src).unwrap();
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse(src_path.to_str().unwrap(), false);
        let lines = p.diagnostics.lines().join("\n");
        assert!(
            p.diagnostics.level() < loft::diagnostics::Level::Error,
            "#690 ({label}): this reuse is idiomatic under loft's flat scoping and must \
             still compile.  Diagnostics: {lines}"
        );
        let _ = fa::remove_file(&src_path);
    }
}
