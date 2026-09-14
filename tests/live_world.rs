// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN162 step 14 — `Disp-World` in the OPEN profile (`LOFT_LIVE_RELOAD=1`), end to end: a
//! RUNNING program whose loop calls an overload set, and the set changed under it.
//!
//! The rule: a specialisation selected in an earlier world must not run once a definition
//! added in a later world would change `Disp-Select` for a call it serves.  Here every call
//! into a set is a per-spelling specialisation — a `__sel_` stub at a static site, a `__dyn_`
//! dispatcher at a dynamic one — rebuilt in the world that invalidates it and swapped in
//! through tier 0's own patch.  The matrix (IMPL.md step 14):
//!
//! (a) a static site at the variants and (b) a dynamic site held at the enum both take an
//!     added more-specific definition from the next round on, and the old answer never
//!     appears again for that pair, while (c) a pair the add does not serve is unchanged;
//! (d) an add that ties a served pair is refused and the world is unchanged;
//! (e) a body edit of the FIRST overload of the name reaches that overload;
//! (f) a re-signatured overload is refused, the last good body serves;
//! (g) a brand-new name is skipped, as tier 0 always did.
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Disk-backed scratch (`target/` lives on disk; `std::env::temp_dir()` is a small tmpfs).
fn test_tmp() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-tmp");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Generous: CI runners are contended, and a round is a few hundred thousand ops.
const WAIT: Duration = Duration::from_secs(40);

const ENUM: &str = "enum Entity {\n  Fireball { n: integer },\n  IceWall { n: integer },\n  Slime { n: integer },\n}\n";

/// The running program: four cells per round.  A = a static site at the variants (Slime,
/// IceWall); B = a dynamic site, two positions held at the enum, holding (Slime, IceWall);
/// C = the control, a dynamic site holding (Fireball, Slime); D = a static site (Fireball,
/// IceWall).  Every definition returns text, so the buffers a text return forwards ride the
/// swap too.
fn program(defs: &str) -> String {
    format!(
        "{ENUM}\n{defs}\nfn spin() -> integer {{\n  s = 0;\n  for i in 0..300000 {{ s += i % 3; }}\n  s\n}}\n\nfn main() {{\n  sl = Slime {{ n: 1 }};\n  iw = IceWall {{ n: 2 }};\n  fb = Fireball {{ n: 3 }};\n  es: Entity = Slime {{ n: 4 }};\n  ew: Entity = IceWall {{ n: 5 }};\n  ef: Entity = Fireball {{ n: 6 }};\n  round = 0;\n  while round < 400 {{\n    z = spin();\n    print(\"round {{round}} {{z}}: A={{hit(sl, iw)}} B={{hit(es, ew)}} C={{hit(ef, es)}} D={{hit(fb, iw)}}\\n\");\n    round += 1;\n  }}\n}}\n"
    )
}

const SET: &str = "fn hit(f: Fireball, w: IceWall) -> text {\n  \"fire-wall {f.n}{w.n}\"\n}\n\nfn hit(a: Entity, b: Entity) -> text {\n  \"any-any {a.n}{b.n}\"\n}\n";

const ADDED: &str = "\nfn hit(s: Slime, w: IceWall) -> text {\n  \"slime-wall {s.n}{w.n}\"\n}\n";

struct Session {
    child: Child,
    prog: PathBuf,
    dir: PathBuf,
    out: Receiver<String>,
    err: Receiver<String>,
    seen_err: Vec<String>,
}

impl Session {
    fn start(name: &str, src: &str) -> Session {
        let dir = test_tmp().join(format!("live_world_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let prog = dir.join("world.loft");
        std::fs::write(&prog, src).unwrap();
        let mut child = Command::new(loft_bin())
            .arg("--interpret")
            .arg("--no-warnings")
            .arg(&prog)
            .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
            .env("LOFT_LIVE_RELOAD", "1")
            .env("LOFT_TIMEOUT", "120")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn loft");
        let (out_tx, out) = channel();
        let stdout = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for l in BufReader::new(stdout).lines().map_while(Result::ok) {
                if out_tx.send(l).is_err() {
                    break;
                }
            }
        });
        let (err_tx, err) = channel();
        let stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            for l in BufReader::new(stderr).lines().map_while(Result::ok) {
                if err_tx.send(l).is_err() {
                    break;
                }
            }
        });
        Session {
            child,
            prog,
            dir,
            out,
            err,
            seen_err: Vec::new(),
        }
    }

    fn edit(&self, src: &str) {
        std::fs::write(&self.prog, src).unwrap();
    }

    /// The next round line whose text satisfies `pred`; every line consumed on the way is
    /// handed to `each`, so a caller can assert what the rounds BEFORE the match said.
    fn round_where(&self, pred: impl Fn(&str) -> bool, mut each: impl FnMut(&str)) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let l = self
                .out
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("no matching round within {WAIT:?}"));
            if pred(&l) {
                return l;
            }
            each(&l);
        }
    }

    /// The next `n` round lines, in order.
    fn rounds(&self, n: usize) -> Vec<String> {
        (0..n).map(|_| self.round_where(|_| true, |_| {})).collect()
    }

    /// Wait for a stderr line containing `needle` (the reload host's report).
    fn report(&mut self, needle: &str) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let l = self.err.recv_timeout(left).unwrap_or_else(|_| {
                panic!(
                    "no stderr line containing {needle:?} within {WAIT:?}; stderr so far:\n{}",
                    self.seen_err.join("\n")
                )
            });
            self.seen_err.push(l.clone());
            if l.contains(needle) {
                return l;
            }
        }
    }

    /// Drain stderr for `for_secs` and return what arrived — for asserting a report did NOT
    /// come, which is only meaningful after the rounds proved the process is alive.
    fn quiet_reports(&mut self, for_secs: u64) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(for_secs);
        let mut got = Vec::new();
        while let Ok(l) = self
            .err
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            self.seen_err.push(l.clone());
            got.push(l);
        }
        got
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn cell<'a>(line: &'a str, name: &str) -> &'a str {
    let key = format!(" {name}=");
    let start = line
        .find(&key)
        .unwrap_or_else(|| panic!("no cell {name} in {line:?}"))
        + key.len();
    let rest = &line[start..];
    let end = rest
        .find(" B=")
        .or_else(|| rest.find(" C="))
        .or_else(|| rest.find(" D="))
        .unwrap_or(rest.len());
    &rest[..end]
}

/// (a) (b) (c): the added definition is selected from the next round on, at the static site
/// and at the dynamic one; the old answer never appears again for that pair; the control pair
/// is untouched.
#[test]
fn an_added_overload_is_selected_by_the_running_loop() {
    let mut s = Session::start("add", &program(SET));
    let before = s.rounds(2);
    for l in &before {
        assert_eq!(
            cell(l, "A"),
            "any-any 12",
            "before the add, the static (Slime, IceWall) site reaches the fallback: {l}"
        );
        assert_eq!(
            cell(l, "B"),
            "any-any 45",
            "before the add, the dynamic site reaches the fallback: {l}"
        );
    }
    s.edit(&format!("{}{ADDED}", program(SET)));
    let report = s.report("added — world 1");
    assert!(
        report.contains("specialisation(s) of 'hit' rebuilt")
            && report.contains("call site(s) patched"),
        "the report names what moved: {report}"
    );
    // The first round selected in world 1.  Rounds compiled before it may still show the
    // old answer (the loop was mid-round); once the new answer appears it must never revert.
    let first = s.round_where(
        |l| cell(l, "A") == "slime-wall 12",
        |l| {
            assert_eq!(
                cell(l, "A"),
                "any-any 12",
                "a round before the swap reads the old world whole: {l}"
            );
        },
    );
    assert_eq!(
        cell(&first, "B"),
        "slime-wall 45",
        "the dynamic site took the new world in the same round: {first}"
    );
    assert_eq!(
        cell(&first, "C"),
        "any-any 64",
        "the control pair (Fireball, Slime) is unchanged: {first}"
    );
    assert_eq!(
        cell(&first, "D"),
        "fire-wall 32",
        "the untouched static site is unchanged: {first}"
    );
    for l in s.rounds(6) {
        assert_eq!(
            cell(&l, "A"),
            "slime-wall 12",
            "a stale specialisation never runs again: {l}"
        );
        assert_eq!(
            cell(&l, "B"),
            "slime-wall 45",
            "a stale specialisation never runs again: {l}"
        );
        assert_eq!(cell(&l, "C"), "any-any 64", "{l}");
    }
}

/// (d): an add that ties a pair the running program serves is refused, naming the tuple, and
/// the world is unchanged — the loop keeps its selections.
#[test]
fn an_add_that_ties_a_served_pair_is_refused() {
    // (Fireball, IceWall) at D and at the dynamic sites is taken by `hit(Fireball, Entity)`;
    // adding `hit(Entity, IceWall)` makes it ambiguous (`Disp-Ambiguous`), so the ADD is
    // refused (Q3) rather than the running call.
    let set = "fn hit(f: Fireball, b: Entity) -> text {\n  \"fire-any {f.n}{b.n}\"\n}\n\nfn hit(a: Entity, b: Entity) -> text {\n  \"any-any {a.n}{b.n}\"\n}\n";
    let mut s = Session::start("tie", &program(set));
    let before = s.rounds(2);
    assert_eq!(cell(&before[1], "D"), "fire-any 32", "{}", before[1]);
    s.edit(&format!(
        "{}\nfn hit(a: Entity, w: IceWall) -> text {{\n  \"any-wall {{a.n}}{{w.n}}\"\n}}\n",
        program(set)
    ));
    let refusal = s.report("is refused");
    assert!(
        refusal.contains("the running world is unchanged"),
        "the refusal says the world did not move: {refusal}"
    );
    assert!(
        s.seen_err
            .iter()
            .any(|l| l.contains("is ambiguous at (Fireball, IceWall)")),
        "the refusal names the tuple the set cannot decide: {:?}",
        s.seen_err
    );
    for l in s.rounds(4) {
        assert_eq!(
            cell(&l, "D"),
            "fire-any 32",
            "the world is unchanged after a refused add: {l}"
        );
        assert_eq!(cell(&l, "A"), "any-any 12", "{l}");
    }
}

/// (e) (f): a body edit of the FIRST overload reaches that overload (the watcher keys blocks
/// by declaration head, not by name); a re-signatured overload is refused and the last good
/// body keeps serving.
#[test]
fn a_body_edit_reaches_its_overload_and_a_resignature_is_refused() {
    let mut s = Session::start("edit", &program(SET));
    let _ = s.rounds(2);
    let edited = SET.replace("\"fire-wall {f.n}{w.n}\"", "\"FIRE-WALL-v2 {f.n}{w.n}\"");
    assert_ne!(edited, SET);
    s.edit(&program(&edited));
    let report = s.report("'hit' v1 live");
    assert!(report.contains("call site(s) patched"), "{report}");
    let first = s.round_where(|l| cell(l, "D") == "FIRE-WALL-v2 32", |_| {});
    assert_eq!(
        cell(&first, "A"),
        "any-any 12",
        "the other overload is untouched by a body edit: {first}"
    );
    // (f) the same overload re-signatured: refused whole, v2 keeps serving.
    let resigned = edited.replace(
        "fn hit(f: Fireball, w: IceWall) -> text",
        "fn hit(f: Fireball, w: IceWall, k: integer) -> text",
    );
    assert_ne!(resigned, edited);
    s.edit(&program(&resigned));
    let refusal = s.report("changed its signature; restart to apply");
    assert!(refusal.contains("'hit'"), "{refusal}");
    for l in s.rounds(3) {
        assert_eq!(
            cell(&l, "D"),
            "FIRE-WALL-v2 32",
            "the last good body keeps serving: {l}"
        );
    }
}

/// (g): a brand-new NAME is skipped, as tier 0 always did — nothing calls it.
#[test]
fn a_new_name_is_still_skipped() {
    let mut s = Session::start("newname", &program(SET));
    let _ = s.rounds(2);
    s.edit(&format!(
        "{}\nfn other(n: integer) -> integer {{\n  n + 1\n}}\n",
        program(SET)
    ));
    let _ = s.rounds(6);
    let got = s.quiet_reports(1);
    assert!(
        got.iter().all(|l| l.contains("watching")),
        "a new name reports nothing and changes nothing: {got:?}"
    );
    for l in s.rounds(2) {
        assert_eq!(cell(&l, "A"), "any-any 12", "{l}");
    }
}
