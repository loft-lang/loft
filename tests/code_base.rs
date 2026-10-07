// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-CodeBase` — the interpreter reads its bytecode through a base pointer and a length
//! cached in `State`, so an operand read is one add rather than a walk through the `Arc` and the
//! `Vec`.  The buffer moves when code is written after it was cached — it grows, or it is copied
//! because a `par` worker still shares it — and `State::edit_code`, the one writer, re-derives
//! both.  What that can break is code written AFTER a run: a stale length cannot reach it (an
//! out-of-range read), a stale base reads the buffer it moved away from.  A REPL session is
//! exactly that shape — each input is compiled onto the end of the code of the inputs before it
//! and then run — so these cells define functions after earlier runs, enough of them that the
//! buffer has to grow, and read each result back.  And the one structural fact the cache rests
//! on — nothing writes the bytecode except through `edit_code` — is checked over the source.
use loft::file_access as fa;
use loft::repl::{Eval, ReplSession};
use std::path::Path;

/// A body long enough that a few dozen of them outgrow any buffer's spare capacity.
fn body(i: usize) -> String {
    let mut b = String::from("t = x;");
    for k in 0..400 {
        b.push_str(&format!(" t = t + {};", (i + k) % 7));
    }
    b
}

/// The value `grow{i}(3)` must answer: 3 plus the 400 increments its body adds.
fn expected(i: usize) -> i64 {
    3 + (0..400).map(|k| ((i + k) % 7) as i64).sum::<i64>()
}

#[test]
fn code_written_after_a_run_is_reached_and_read_right() {
    let mut s = ReplSession::new("default").expect("load stdlib");
    for i in 0..48 {
        let def = format!("fn grow{i}(x: integer) -> integer {{ {} t }}", body(i));
        assert!(matches!(s.eval(&def), Eval::Ran), "definition {i} compiles");
        // Every earlier function, too: a stale base reads the moved-away buffer for ALL code,
        // not only the newest.
        for j in [0, i / 2, i] {
            let want = expected(j).to_string();
            assert_eq!(
                s.value_of(&format!("grow{j}(3)")).as_deref(),
                Some(want.as_str()),
                "grow{j}(3) after {i} definitions"
            );
        }
    }
}

/// Nothing writes the bytecode except `State::edit_code`, which re-derives the cache.
#[test]
fn the_bytecode_is_written_only_by_its_writer() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sites = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in fa::read_dir(&dir).expect("read src") {
            let path = entry.os_spelling();
            if fa::is_dir(&path) {
                stack.push(path);
            } else if fa::has_extension(&path, "rs") {
                let text = fa::read_to_string(&path).expect("read file");
                for (n, line) in text.lines().enumerate() {
                    let code = line.trim_start();
                    if code.starts_with("//") {
                        continue;
                    }
                    if code.contains("make_mut(&mut self.bytecode)")
                        || code.contains("make_mut(&mut state.bytecode)")
                        || code.contains(".bytecode = ")
                        || code.contains("bytecode.as_mut_ptr()")
                    {
                        sites.push(format!("{}:{}", loft::file_access::portable(&path), n + 1));
                    }
                }
            }
        }
    }
    assert_eq!(
        sites.len(),
        1,
        "the bytecode is written outside `State::edit_code`, so the cached base and length \
         (@FR-R-CodeBase) would not follow it: {sites:?}"
    );
    assert!(
        sites[0].contains("state/mod.rs"),
        "the one write is the writer's: {sites:?}"
    );
}
