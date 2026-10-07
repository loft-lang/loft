// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `advice[function-complexity]` — who it reaches.
//!
//! The cure this advice names is "lift the innermost part at line N into its own function",
//! which is an edit to that function's OWN source. A consumer importing a library cannot make
//! it, so firing there spends the reader's attention on code they do not own — and an advice
//! that cannot be acted on is one people learn to scroll past, which costs more than the
//! silence it replaced.
//!
//! It was reaching them. Three of the six libraries bundled in `lib/` printed one into every
//! program that `use`d them, the Lexer chapter of the language reference among them: running
//! the reference's own example emitted a nudge about `lib/lexer.loft`'s internals.
//!
//! `Data::source_is_owned` is the existing answer and is RELATIVE to what is being built — a
//! library's own source is owned when its author compiles it, and a dependency when a consumer
//! imports it. So the same function is loud for the person who can fix it and quiet for the
//! person who cannot. `advise_group_apart` already gated on it; this one did not.
//!
//! Both halves are here on purpose. Without the firing cell the silent one is vacuous — a lint
//! that never fires at all would pass it — and without the author cell the fix is
//! indistinguishable from deleting the diagnostic.
//!
//! Binary-invoked like `tests/group_apart_lint.rs`: these are end-to-end compile diagnostics on
//! stderr. `LOFT_NO_CACHE` because the warm program cache skips the re-parse that produces them.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

const CODE: &str = "advice[function-complexity]";

/// The function the advice is about: five nested loops around five nested ifs.
const KNOTTY: &str = "pub fn knotty(n: integer) -> integer {\n\
  \x20 x = 0;\n\
  \x20 for a in 0..n { for b in 0..n { for c in 0..n { for d in 0..n { for e in 0..n {\n\
  \x20   if a > 0 { if b > 0 { if c > 0 { if d > 0 { if e > 0 { x += 1 } } } } }\n\
  \x20 } } } } }\n\
  \x20 x\n\
}\n";

fn probe_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("loft_complexity_lint_ownership");
    fa::create_dir_all(&dir).expect("probe dir");
    dir
}

fn run(path: &PathBuf, extra: &[&str]) -> String {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--interpret");
    for a in extra {
        cmd.arg(a);
    }
    let out = cmd
        .arg(path)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn the_author_of_the_code_is_told() {
    // The control. Every silent case below is vacuous without one that fires, and this also
    // pins that the fix did not simply switch the diagnostic off.
    let dir = probe_dir();
    let path = dir.join("author_view.loft");
    fa::write(
        &path,
        format!("{KNOTTY}fn main() {{ print(\"{{knotty(2)}}\"); }}\n"),
    )
    .expect("write probe");
    let err = run(&path, &[]);
    assert!(
        err.contains(CODE) && err.contains("knotty"),
        "compiling the code directly must still advise its author — stderr was:\n{err}"
    );
}

#[test]
fn a_consumer_importing_it_is_not() {
    // The same function, reached through `use`. Nothing the consumer can write changes it.
    let dir = probe_dir();
    let libdir = dir.join("lib");
    fa::create_dir_all(&libdir).expect("lib dir");
    fa::write(libdir.join("knotlib.loft"), KNOTTY).expect("write lib");

    let path = dir.join("consumer.loft");
    fa::write(
        &path,
        "use knotlib;\nfn main() { print(\"{knotlib::knotty(2)}\"); }\n",
    )
    .expect("write probe");

    let err = run(&path, &["--lib", libdir.to_str().expect("utf-8 lib path")]);
    assert!(
        !err.contains(CODE),
        "a consumer cannot lift a line out of someone else's library, so the nudge must not \
         reach them — stderr was:\n{err}"
    );
}

#[test]
fn the_consumers_own_complexity_still_reaches_them() {
    // The sharp cell: importing a library must not buy silence about the consumer's OWN code.
    // A gate written one step too wide — quiet whenever any library is loaded — passes both
    // tests above and fails this one.
    let dir = probe_dir();
    let libdir = dir.join("lib");
    fa::create_dir_all(&libdir).expect("lib dir");
    fa::write(libdir.join("knotlib.loft"), KNOTTY).expect("write lib");

    let path = dir.join("consumer_own.loft");
    fa::write(
        &path,
        format!(
            "use knotlib;\n\
             {}\
             fn main() {{ print(\"{{mine(2)}} {{knotlib::knotty(2)}}\"); }}\n",
            KNOTTY.replace("knotty", "mine")
        ),
    )
    .expect("write probe");

    let err = run(&path, &["--lib", libdir.to_str().expect("utf-8 lib path")]);
    assert!(
        err.contains(CODE) && err.contains("mine"),
        "the consumer's own knotted function is theirs to fix, so it must still be named — \
         stderr was:\n{err}"
    );
    assert!(
        !err.contains("knotty"),
        "and the library's must not be, in the same run — stderr was:\n{err}"
    );
}
