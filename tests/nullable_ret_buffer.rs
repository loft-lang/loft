// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! loft#938 — the return buffer for a NULLABLE COLLECTION return. **Default ON.**
//!
//! `-> vector<T>?` gets the same hidden `__retbuf` the non-nullable form always had, so a
//! nullable collection result is delivered into the caller's buffer instead of the callee
//! allocating a store the caller never frees.
//!
//! It shipped off for months behind `LOFT_NULLABLE_RETBUF` because one shape stayed wrong: an
//! enum-dispatched arm binding the inner call to a local and returning it. That shape was two
//! stacked defects, and each hid the other — fixing either alone only moved the damage:
//!
//!  1. **the caller's dep translation.** `call_dependencies` matched BARE shapes only, so an
//!     `Optional` return fell through and kept the callee's ATTRIBUTE indices; read in the
//!     caller those are frame variable numbers, so the result was typed as borrowing whichever
//!     local held that number. Alone, this turned a use-after-free into a leak.
//!  2. **the callee's delivery.** `fresh_owned_vector_deps` matched `Type::Vector` on the
//!     RETURNED LOCAL's own type, and `v = src(i); return v;` types `v` as `Optional(Vector)`.
//!     So `block_result`'s tail intercept never fired, `ref_return` was never reached at all,
//!     and the arm handed back its own store while the caller's buffer went untouched.
//!
//! Both are the same wrapper mistake at two removes. Six gates had already been peeled when
//! the switch was built; gate 7 outlived that sweep because it is on the returned VALUE rather
//! than on the return TYPE.
//!
//! `LOFT_NO_NULLABLE_RETBUF=1` / `LOFT_NO_OPTIONAL_DEP_PEEL=1` restore the old behaviour — the
//! A/B on one binary, and what `default_path_is_unchanged` pins.
//!
//! Two method notes, both earned here.
//!
//! `LOFT_TRACE_RETPROMO=1` FIRST. It prints an ENTER line per `ref_return` call AND a verdict
//! line per candidate, and the distinction is the point: no verdict means the classifier said
//! nothing, no ENTER means `ref_return` was never called. Those are different bugs in different
//! files, and gate 7 was the second. The trace carried only verdicts while that gate was open,
//! so silence could not be read — which is most of why it took three sessions.
//!
//! `LOFT_STRICT_STORES=1` SECOND, and not only when something looks wrong. Every value here is
//! correct in every state of the bug; the fault appears only as a store used after its free, or
//! as one never freed. A green run without the oracle is not evidence, and a green run that
//! checks only ONE of those two is not either — a UAF-only assertion went green the moment
//! gate 1 landed, while the store had simply stopped being freed at all.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test binary path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("loft")
}

/// Run `src` on `backend`, with the switch on or off. Returns `(stdout, stderr)`.
///
/// `tag` names the probe's own file. Tests in one binary share a PID, so keying the scratch
/// path on that alone let them clobber each other's source — a failure that reproduced only
/// in the batch and passed when the test was run alone.
fn run(src: &str, backend: &str, retbuf: bool, tag: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("loft938_{}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join(format!("{tag}.loft"));
    fa::write(&file, src).expect("write probe");
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend)
        .arg(&file)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1");
    if !retbuf {
        // The buffer is ON by default now, so the `false` arm DISABLES it — the
        // before-half of the A/B, and what `default_path_is_unchanged` pins.
        cmd.env("LOFT_NO_NULLABLE_RETBUF", "1")
            .env("LOFT_NO_OPTIONAL_DEP_PEEL", "1");
    }
    let out = cmd.output().expect("invoke loft");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn leaked(stderr: &str) -> bool {
    stderr.contains("not freed")
}

/// The filed shape: an unbound `f(i) != null` in a loop. The result is never bound, so the
/// comparison captures it in a work-ref whose free is emitted ONCE at scope exit while each
/// turn overwrites the slot. 39 of 40 stores are orphaned, and the count scales with the
/// iteration count — the unbounded shape of loft#688.
const FILED: &str = r#"
fn gives(n: integer) -> vector<integer>? { if n != 0 { [n] } else { null } }
fn main() {
  c = 0;
  for i in 0..40 { if gives(i + 1) != null { c += 1; } }
  println("c={c}");
}
"#;

/// A `-> vector<T>?` returning a BORROWED view of a parameter. The constraint on any fix: the
/// caller's collection must survive, so this must not become an over-free once the leak above
/// is closed. It is the row that says "free unconditionally" is not the answer.
const BORROWED: &str = r#"
fn view(v: vector<integer>, n: integer) -> vector<integer>? { if n != 0 { v } else { null } }
fn main() {
  base = [71, 82, 93];
  c = 0;
  for i in 0..40 { if view(base, i + 1) != null { c += 1; } }
  println("c={c} base={len(base)} base0={base[0]}");
}
"#;

/// One function whose arms disagree: a `null` arm, an arm aliasing a PARAMETER, and an arm
/// allocating a FRESH store. The null arm forces the `__ret_N` merge before promotion sees
/// the arms, so none of them is a return tail and none delivers into the buffer.
const MIXED: &str = r#"
fn pick(v: vector<integer>, n: integer) -> vector<integer>? {
  if n == 0 { null } else if n == 1 { v } else { [n, n + 1] }
}
fn main() {
  base = [71, 82, 93];
  c = 0;
  for i in 0..40 { r = pick(base, i); c += len(r ?? []); }
  println("c={c} base={len(base)} base0={base[0]}");
}
"#;

#[test]
fn the_filed_leak_is_fixed() {
    let (out, err) = run(FILED, "--interpret", true, "filed_on");
    assert!(out.contains("c=40"), "value changed: {out}{err}");
    assert!(!leaked(&err), "loft#938's filed leak is back\n{err}");
}

#[test]
fn a_borrowed_view_is_not_over_freed() {
    let (out, err) = run(BORROWED, "--interpret", true, "borrowed_on");
    assert!(
        out.contains("c=40 base=3 base0=71"),
        "the caller's collection did not survive — a free became an over-free: {out}{err}"
    );
    assert!(!leaked(&err), "borrowed view leaked instead\n{err}");
}

#[test]
fn default_path_is_unchanged() {
    // The A/B in the other direction, and what it exists for: with the mechanism DISABLED the
    // damage returns, so the greens above are this switch's doing rather than something
    // else's. Without a live before-half they prove nothing.
    //
    // ⚠ It is measured on MIXED and on the VALUE channel, and it used to be measured on FILED
    // and on the leak channel. FILED stopped being a witness when loft#1329 gave `owned_ref`
    // its `Optional` peel: the work-ref that program leaks through is an `Optional(Vector)`
    // LOCAL re-Set every turn, so the displaced free now releases it whatever this switch
    // says. That is the second cure this test's old message named as one of its two
    // explanations, and it is the true one — FILED is clean in BOTH states now.
    //
    // MIXED is the right witness because the switch is the ONLY thing standing between it and
    // a wrong answer: with the buffer off, `pick`'s aliasing arm hands back the caller's own
    // `base` and the result is read as owned, so `base` is destroyed mid-loop and the program
    // prints `base=2 base0=39` instead of `base=3 base0=71`. Measured identically on
    // `a8c0b74d`, before the peel existed — this is the switch's own effect, not a regression
    // the peel introduced. A value channel is also the stronger one: a leak gate is monotone
    // and cannot score an over-free, which is exactly what the off-half does here.
    let (out, err) = run(MIXED, "--interpret", false, "mixed_off");
    assert!(
        out.contains("c=79"),
        "the count changed with the mechanism off, so this no longer isolates the \
         delivery: {out}{err}"
    );
    assert!(
        out.contains("base=2 base0=39"),
        "LOFT_NO_NULLABLE_RETBUF no longer restores the pre-fix path, so the green tests \
         above prove nothing — either the opt-out broke, or the aliasing arm acquired a \
         second cure the way FILED's leak did:\n{out}{err}"
    );
}

/// One function whose arms disagree — a `null` arm, an arm aliasing a PARAMETER and an arm
/// allocating a FRESH store — is the shape that killed the signature-level
/// allocates-vs-aliases flag. The buffer ABI removes the variance instead of deciding it.
#[test]
fn mixed_arms_deliver_into_the_buffer() {
    let (out, err) = run(MIXED, "--interpret", true, "mixed_on");
    assert!(
        out.contains("c=79 base=3 base0=71"),
        "value or container changed: {out}{err}"
    );
    assert!(!leaked(&err), "the fresh arms leak again\n{err}");
}

/// A FORWARDING tail — `fn fwd(i) -> vector<T>? { a1(i) }` — is what the delivery selector in
/// `block_result` never reached, so it compiled to `return null` on `--native` while the
/// interpreter read the freed slot and only looked right.
const FORWARD: &str = r#"
fn a1(i: integer) -> vector<integer>? { if i == 0 { return null } return [i, i + 10]; }
fn tail(i: integer) -> vector<integer>? { a1(i) }
fn expl(i: integer) -> vector<integer>? { return a1(i); }
fn main() {
  c = 0;
  for i in 0..40 { r = tail(i + 1); c += len(r ?? []); }
  for i in 0..40 { r = expl(i + 1); c += len(r ?? []); }
  n = 0;
  if tail(0) == null { n += 1; }
  if expl(0) == null { n += 1; }
  println("c={c} n={n}");
}
"#;

#[test]
fn forwarding_return_delivers_and_keeps_null() {
    for backend in ["--interpret", "--native"] {
        let tag = format!("forward_{}", backend.trim_start_matches('-'));
        let (out, err) = run(FORWARD, backend, true, &tag);
        assert!(
            out.contains("c=160 n=2"),
            "{backend}: a forwarded nullable collection lost its value or its null: {out}{err}"
        );
        assert!(
            !leaked(&err),
            "{backend}: the forwarded store leaked\n{err}"
        );
    }
}

/// The whole boundary matrix, as a runnable program. It lives under `tests/probes/` rather than
/// `tests/scripts/` on purpose: the suite runs `tests/scripts/` on the DEFAULT path, where this
/// program is expected to fail — it is the regression guard for a bug the default still has.
#[test]
fn the_boundary_matrix_passes_on_both_backends() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/probes/938-nullable-collection-return-buffer.loft");
    for backend in ["--interpret", "--native"] {
        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_TIMEOUT", "300")
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_NULLABLE_RETBUF", "1")
            .output()
            .expect("invoke loft");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(all.contains("938 ok"), "{backend}:\n{all}");
        assert!(!leaked(&all), "{backend} leaked:\n{all}");
    }
}

#[test]
fn native_optional_unify_compiles_correctly() {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/scripts/pln133-optional-unify.loft");
    for backend in ["--interpret", "--native"] {
        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_TIMEOUT", "300")
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_NULLABLE_RETBUF", "1")
            .output()
            .expect("invoke loft");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        // A method returning `vector<T>?` through different routes used to read its element
        // back as 0 where 14 is correct, and only on `--native`.
        assert!(
            all.contains("pln133-optional-unify ok"),
            "{backend} miscompiles a nullable-collection return under the switch\n{all}"
        );
        assert!(!leaked(&all), "{backend} leaked:\n{all}");
    }
}

/// An enum-dispatched `-> vector<T>?` whose implementation BINDS the inner call to a local
/// and returns that local. Every value is correct; `LOFT_STRICT_STORES=1` is the only witness.
///
/// # The filed scope was wrong twice, and both corrections matter
///
/// It was filed as needing BOTH a two-arm dispatch AND two call sites. Neither is the axis.
/// **One site and one arm reproduce it** — the probe below is that minimal shape, and the
/// two-site form it replaced was a strictly larger program answering the same way. What the
/// shape actually needs is the enum dispatch plus the *local-binding* implementation:
/// `take(VD)`, which returns the inner call DIRECTLY, is clean in exactly the same program.
///
/// And it was filed as one bug. It is two, stacked, which is why fixing either alone looks
/// like a regression:
///
/// 1. **The dep translation** — FIXED. `call_dependencies` matched only BARE shapes, so an
///    `Optional` return fell through its `else { tp }` and kept the callee's ATTRIBUTE
///    indices; read in the caller those are frame variable numbers. Peeling the `Optional`
///    (`LOFT_NO_OPTIONAL_DEP_PEEL=1` restores the old path) makes each site name its own
///    buffer — `__ref_3` / `__ref_4` rather than both naming `gd`.
///
/// 2. **The delivery** — OPEN, and what this test now pins. `t_2VL_take_v` never writes the
///    `__retbuf` it is given. It allocates a store of its own, fills THAT, and returns it:
///
///    ```text
///    fn t_2VL_take_v(cell, var_self, var_i, var___retbuf) -> DbRef {
///        var___ref_1 = OpDatabase(cell, var___ref_1, 22_i32);   // its OWN store
///        let mut var_v = n_vec_src(cell, var_i, …, var___ref_1); // fills that
///        return var_v                                            // hands that back
///    }
///    ```
///
///    `LOFT_TRACE_RETPROMO=1` prints NO line for it at all, which per `keys::nullable_ret_buffer`
///    means a gate UPSTREAM of `classify_ret_promotion` — the local `v` is never offered for
///    promotion. Its sibling `t_2VD_take_v` gets `Bind { buf_attr: 2, substitute: true }`.
///
/// So the caller's dep and the callee's delivery disagree, and whichever one you believe, the
/// other is wrong: with the stale dep the store is freed on an unrelated local's schedule
/// (`USE AFTER FREE`), and with the correct dep the caller frees an empty buffer while the
/// real store has no owner (`NEVER FREED`). Fixing (1) alone converts one into the other,
/// which is why this test asserts BOTH — a leak-blind version of it went green on a change
/// that had only moved the damage.
///
/// The remaining work is (2): reach the classifier for a returned local on this route, so the
/// value really is in the buffer the dep now correctly names. That is the last thing between
/// the switch and the default.
const TWO_SITE_DISPATCH: &str = r#"
struct VD { step: integer }
struct VL { step: integer }
fn src(i: integer, step: integer) -> vector<integer>? { if i == 0 { return null } return [i, i + step]; }
pub fn take(self: VD, i: integer) -> vector<integer>? { return src(i, self.step); }
pub fn take(self: VL, i: integer) -> vector<integer>? { v = src(i, self.step); return v; }
enum Any { AD { d: VD }, AL { l: VL } }
pub fn take(self: Any, i: integer) -> vector<integer>? {
  match self { AD { d } => d.take(i), AL { l } => l.take(i) }
}
fn main() {
  vl: Any = AL { l: VL { step: 20 } };
  gl = vl.take(6) ?? [];
  println("gl={gl[1] ?? -1}");
}
"#;

#[test]
fn dispatch_arm_returning_a_local_delivers_into_the_buffer() {
    let dir = std::env::temp_dir().join(format!("loft938_{}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join("two_site.loft");
    fa::write(&file, TWO_SITE_DISPATCH).expect("write probe");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&file)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_STRICT_STORES", "1")
        .output()
        .expect("invoke loft");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // The VALUE is right, which is the trap: only the strict-store oracle sees it.
    assert!(all.contains("gl=26"), "values changed:\n{all}");
    // BOTH halves, and that is the point of this test rather than a detail of it: the caller's
    // dep and the callee's delivery disagree, so believing either one alone leaves the other
    // wrong. A UAF-only assertion passed the moment the dep was fixed, while the store had
    // simply stopped being freed at all.
    assert!(
        !all.contains("USE AFTER FREE"),
        "the result is still typed as borrowing an unrelated local\n{all}"
    );
    assert!(
        !all.contains("NEVER FREED"),
        "the arm still returns its own store while the caller frees an empty __retbuf\n{all}"
    );
}
