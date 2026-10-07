// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN13 phase 6 (diagnostics slice) — an unresolved bare call names the
//! package that provides it.
//!
//! Calling a library's free function without a `use` used to end at
//! `Unknown function rand` with no route forward, even though the registry index
//! knows exactly which package exports `rand`.  Bare calls still do not RESOLVE
//! (that is the rest of phase 6, and it has to settle stdlib shadowing first);
//! this only replaces the dead end with the two ways to say what was meant.
//!
//! Every case runs against a FAKE `LOFT_HOME` holding a hand-written index, so
//! the assertions never depend on what the developer happens to have cached.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    workspace_root().join("target/release/loft")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A cached index exporting `rand` (free fn, package `random`), `matches`
/// (a METHOD — must never be offered for a bare call) and `dup` from two
/// packages at once.
const INDEX: &str = r#"{
  "schema_version": 1,
  "updated": "2026-07-23",
  "packages": {
    "random": {
      "name": "random",
      "versions": {
        "0.3.0": {
          "version": "0.3.0",
          "url": "https://example.invalid/random-0.3.0.tar.gz",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
          "size": 1,
          "loft": "2026.7.2",
          "published": "2026-07-23",
          "api": [
            { "sig": "pub fn rand(low: integer, high: integer) -> integer", "doc": "A number." },
            { "sig": "pub fn matches(self: text, pattern: text) -> boolean", "doc": "A method." },
            { "sig": "pub struct RandStream { s1: integer }", "doc": "A stream." },
            { "sig": "pub value struct Seed { v: integer }", "doc": "A seed." }
          ]
        }
      }
    },
    "dicer": {
      "name": "dicer",
      "versions": {
        "0.1.0": {
          "version": "0.1.0",
          "url": "https://example.invalid/dicer-0.1.0.tar.gz",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
          "size": 1,
          "loft": "2026.7.2",
          "published": "2026-07-23",
          "api": [
            { "sig": "pub fn dup(v: integer) -> integer", "doc": "Also here." },
            { "sig": "pub struct Die { sides: integer }", "doc": "A die." }
          ]
        }
      }
    },
    "duper": {
      "name": "duper",
      "versions": {
        "0.1.0": {
          "version": "0.1.0",
          "url": "https://example.invalid/duper-0.1.0.tar.gz",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
          "size": 1,
          "loft": "2026.7.2",
          "published": "2026-07-23",
          "api": [
            { "sig": "pub fn dup(v: integer) -> integer", "doc": "And here." },
            { "sig": "pub enum Die { D4, D6 }", "doc": "Dice kinds." }
          ]
        }
      }
    },
    "shaper": {
      "name": "shaper",
      "versions": {
        "0.9.0": {
          "version": "0.9.0",
          "url": "https://example.invalid/shaper-0.9.0.tar.gz",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
          "size": 1,
          "loft": "2026.7.2",
          "published": "2026-07-23",
          "api": [ { "sig": "pub struct OldShape { a: integer }", "doc": "Dropped in 0.10." } ]
        },
        "0.10.0": {
          "version": "0.10.0",
          "url": "https://example.invalid/shaper-0.10.0.tar.gz",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
          "size": 1,
          "loft": "2026.7.2",
          "published": "2026-07-24",
          "api": [ { "sig": "pub struct NewShape { a: integer }", "doc": "Added in 0.10." } ]
        }
      }
    }
  }
}"#;

/// Compile `src` against a fake registry home and return the diagnostics.
fn diagnostics_for(tag: &str, src: &str, with_index: bool) -> String {
    let pid = std::process::id();
    let home = std::env::temp_dir().join(format!("loft_fnhint_{tag}_{pid}"));
    let _ = fa::remove_dir_all(&home);
    fa::create_dir_all(home.join(".loft/registry")).expect("mkdir registry");
    if with_index {
        fa::write(home.join(".loft/registry/index.json"), INDEX).expect("write index");
    }
    let prog = home.join("prog.loft");
    fa::write(&prog, src).expect("write program");

    let out = Command::new(loft_bin())
        .args(["--interpret", "--errors=compact"])
        .arg(&prog)
        .env("LOFT_HOME", &home)
        .output()
        .expect("invoke loft");
    let text =
        String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    let _ = fa::remove_dir_all(&home);
    text
}

#[test]
fn unresolved_bare_call_names_the_package_that_provides_it() {
    let d = diagnostics_for("one", "fn main() {\n    v = rand(1, 100);\n}\n", true);
    assert!(
        d.contains("random::rand"),
        "the message should point at the qualified call; got:\n{d}"
    );
    assert!(
        d.contains("use random::(rand);"),
        "the message should offer the `use` form too; got:\n{d}"
    );
}

/// A published METHOD is not something a bare call could have meant — and it
/// already resolves without a `use` through the lazy-load triggers — so it must
/// not be offered here.
#[test]
fn a_published_method_is_not_offered_for_a_bare_call() {
    let d = diagnostics_for(
        "method",
        "fn main() {\n    v = matches(\"a\", \"b\");\n}\n",
        true,
    );
    assert!(
        !d.contains("random::matches"),
        "a method must not be suggested as a bare qualified call; got:\n{d}"
    );
}

/// Two packages exporting the same name: name both rather than pick one.
#[test]
fn an_ambiguous_name_lists_every_provider() {
    let d = diagnostics_for("dup", "fn main() {\n    v = dup(2);\n}\n", true);
    assert!(
        d.contains("`dicer`") && d.contains("`duper`"),
        "both providers should be named; got:\n{d}"
    );
}

/// No cached index (a fresh machine, or a registry-less build): the diagnostic
/// degrades to the plain message instead of erroring or stalling.
#[test]
fn no_cached_index_still_reports_the_plain_error() {
    let d = diagnostics_for("noidx", "fn main() {\n    v = rand(1, 100);\n}\n", false);
    assert!(
        d.contains("Unknown function rand"),
        "the plain error must survive; got:\n{d}"
    );
    assert!(
        !d.contains("provides it"),
        "no index means no hint; got:\n{d}"
    );
}

// ── loft#789: the advice must be about the packages the BUILD resolved ────────

/// Compile `src` against a fake registry home AND a local `--lib` package of
/// the same name as one the index knows.
///
/// The collision is the whole point: `random` here is a different package that
/// happens to share a name with the published one, which is exactly what a
/// consumer developing against an unpublished copy has.
fn diagnostics_with_local_random(tag: &str, src: &str) -> String {
    let pid = std::process::id();
    let home = std::env::temp_dir().join(format!("loft_fnhint_{tag}_{pid}"));
    let _ = fa::remove_dir_all(&home);
    fa::create_dir_all(home.join(".loft/registry")).expect("mkdir registry");
    fa::write(home.join(".loft/registry/index.json"), INDEX).expect("write index");

    // A LOCAL `random` with a different API — no `rand`.
    let pkg = home.join("libs/random/src");
    fa::create_dir_all(&pkg).expect("mkdir pkg");
    fa::write(
        home.join("libs/random/loft.toml"),
        "[package]\nname = \"random\"\nversion = \"0.0.1\"\nloft = \">=0.1\"\n\n\
         [library]\nentry = \"src/random.loft\"\n",
    )
    .expect("write manifest");
    fa::write(
        pkg.join("random.loft"),
        "pub fn something_else() -> integer { return 1 }\n",
    )
    .expect("write lib");

    let prog = home.join("prog.loft");
    fa::write(&prog, src).expect("write program");
    let out = Command::new(loft_bin())
        .args(["--interpret", "--errors=compact", "--lib"])
        .arg(home.join("libs"))
        .arg(&prog)
        .env("LOFT_HOME", &home)
        .output()
        .expect("invoke loft");
    let text =
        String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    let _ = fa::remove_dir_all(&home);
    text
}

/// loft#789 — when the suggested package is one this build already RESOLVED,
/// the advice must not be "add `use random::(rand);`".
///
/// The file's first line is `use random;`. Following the old advice changed
/// nothing, because the resolved `random` came from `--lib` and has no `rand` —
/// the index is describing a different package of the same name. Sending the
/// author to check an import that is already correct is a poor first suggestion
/// when the real answer is *two packages share a name*.
#[test]
fn advice_does_not_send_you_to_import_what_is_already_imported() {
    let d = diagnostics_with_local_random(
        "resolved",
        "use random;\n\nfn main() {\n    v = rand(1, 100);\n}\n",
    );
    assert!(
        !d.contains("add `use random::(rand);`"),
        "the file already imports it; got:\n{d}"
    );
    assert!(
        d.contains("different packages of the same name"),
        "the message must name the real diagnosis; got:\n{d}"
    );
    assert!(
        d.contains("does not have it"),
        "and say that the resolved package lacks the function; got:\n{d}"
    );
}

/// loft#789 control — with NO local package of that name, the original advice is
/// still the right advice and must be unchanged.
#[test]
fn the_plain_import_advice_survives_when_nothing_collides() {
    let d = diagnostics_for("plain", "fn main() {\n    v = rand(1, 100);\n}\n", true);
    assert!(
        d.contains("add `use random::(rand);`"),
        "an unimported package is still the answer; got:\n{d}"
    );
}

// ── The type half: an un-imported TYPE names its package too ─────────────────

/// A type written in a signature, a construction and a value struct each name the package
/// that declares it — the same advice an un-imported function already had.  Before, `Rect`
/// read "did you mean 'text'?" and `Canvas` a bare "Undefined type".
#[test]
fn an_unresolved_type_names_the_package_that_provides_it() {
    for (tag, src, head) in [
        (
            "sig",
            "fn f(r: RandStream) -> integer { 1 }\nfn main() {}\n",
            "Undefined type RandStream",
        ),
        (
            "construct",
            "fn main() {\n    r = RandStream { s1: 1 };\n}\n",
            "unknown type RandStream",
        ),
        (
            "value",
            "fn f(s: Seed) -> integer { 1 }\nfn main() {}\n",
            "Undefined type Seed",
        ),
    ] {
        let d = diagnostics_for(&format!("type_{tag}"), src, true);
        let name = head.rsplit(' ').next().unwrap_or("");
        assert!(
            d.contains(head)
                && d.contains("the `random` package provides it")
                && d.contains(&format!("use random::({name});")),
            "{tag}: the message names the package and the import; got:\n{d}"
        );
    }
}

/// Two packages declaring a type of that name are both named; one declares a struct and
/// the other an enum, which the lookup reads alike.
#[test]
fn an_ambiguous_type_lists_every_provider() {
    let d = diagnostics_for(
        "type_dup",
        "fn f(d: Die) -> integer { 1 }\nfn main() {}\n",
        true,
    );
    assert!(
        d.contains("`dicer`") && d.contains("`duper`"),
        "both providers should be named; got:\n{d}"
    );
}

/// The NEWEST version decides — by version, not by the index map's string order, which
/// sorts `0.10.0` before `0.9.0` and so took `0.9.0` for the newest.
#[test]
fn only_the_newest_version_by_semver_is_offered() {
    let d = diagnostics_for(
        "type_new",
        "fn f(s: NewShape) -> integer { 1 }\nfn main() {}\n",
        true,
    );
    assert!(
        d.contains("the `shaper` package provides it"),
        "0.10.0 is the newest and declares it; got:\n{d}"
    );
    let d = diagnostics_for(
        "type_old",
        "fn f(s: OldShape) -> integer { 1 }\nfn main() {}\n",
        true,
    );
    assert!(
        !d.contains("provides it"),
        "only 0.9.0 declared it, and 0.9.0 is not the newest; got:\n{d}"
    );
}

/// A misspelling no package declares keeps its "did you mean" — the registry is asked
/// only for a name that matches a declaration exactly.
#[test]
fn a_type_typo_keeps_its_suggestion() {
    let d = diagnostics_for(
        "type_typo",
        "struct Mine { a: integer }\nfn f(m: Minee) -> integer { 1 }\nfn main() {}\n",
        true,
    );
    assert!(
        d.contains("did you mean 'Mine'") && !d.contains("provides it"),
        "the typo suggestion survives; got:\n{d}"
    );
}
