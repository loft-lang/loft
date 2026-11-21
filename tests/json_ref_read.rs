// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-F-Read-Ref`, `@FR-H-Removed` — a pointer read from a document names a record of the
//! result that is there when the read ends and has the type the reference names; any other
//! pointer is a misfit and its reference stays null.
//!
//! A document is text from elsewhere, so each cell is a document the writer never produces.
//! Varied: what the pointer names — a record of the reference's type, a record of another
//! type, a member the document states twice (the pointer at it, and under it), nothing, a
//! `$refs` target — and what happens to the record after the read (kept, removed).  Held
//! fixed: one reference field per document, read through `T.parse`.
//!
//! Values, by hand:
//!
//! | document | `pick` |
//! |---|---|
//! | `#/bigs/0`, a `Big` | that record: `a` reads `hi` |
//! | `#/smalls/0`, a `Small`, for a `reference<Big>` | null — another type |
//! | `next` stated twice, `#/next` | the second `next`: the first is replaced |
//! | `next` stated twice, `#/next/next`, which only the first had | null — replaced |
//! | `#/leaves/4` of an empty list | null — no record |
//! | `#/$refs/0` | the target read from `$refs` |
//! | `#/leaves/1`, then the list is emptied and refilled | null — its record is removed |
//!
//! Without the rule the second row read a `Small`'s number as a text handle (the compiled
//! build died following it), the fourth named a freed record whose text read null, and the
//! last named whatever record took the removed one's place.

use loft::file_access as fa;

const PROGRAM: &str = r#"struct Small { n: integer }
struct Big { a: text, c: vector<integer> }
struct Doc { smalls: vector<Small>, bigs: vector<Big>, pick: reference<Big>? }
struct Node { name: text, next: Node?, pick: reference<Node>? }
struct Leaf { name: text, n: integer }
struct Tree { leaves: vector<Leaf>, pick: reference<Leaf>? }

fn q(s: text) -> text { s.replace("<", "{{").replace(">", "}}").replace("'", "\"") }

fn main() {
  a = Doc.parse(q("<'smalls':[<'n':7>],'bigs':[<'a':'hi','c':[1,2]>],'pick':<'$ref':'#/bigs/0'>>"));
  assert(a.pick != null, "a pointer at a record of the reference's type is read");
  assert(a.pick.a == "hi" && len(a.pick.c) == 2, "and names that record: {a.pick.a}");

  b = Doc.parse(q("<'smalls':[<'n':1094795585>],'bigs':[],'pick':<'$ref':'#/smalls/0'>>"));
  assert(b.pick == null, "a pointer at a record of another type names nothing");

  c = Node.parse(q("<'name':'top','next':<'name':'first'>,'next':<'name':'second'>,'pick':<'$ref':'#/next'>>"));
  assert(c.pick != null, "a pointer at a member stated twice is read");
  assert(c.pick.name == "second", "and names the one the document ends with: {c.pick.name}");

  d = Node.parse(q("<'name':'top','next':<'name':'a','next':<'name':'inner'>>,'next':<'name':'b'>,'pick':<'$ref':'#/next/next'>>"));
  assert(d.pick == null, "a pointer under the first statement names nothing: it was replaced");
  assert(d.next.name == "b", "the member holds its second statement: {d.next.name}");

  e = Tree.parse(q("<'leaves':[],'pick':<'$ref':'#/leaves/4'>>"));
  assert(e.pick == null, "a pointer at no record names nothing");

  f = Tree.parse(q("<'leaves':[],'pick':<'$ref':'#/$refs/0'>,'$refs':[<'name':'out','n':5>]>"));
  assert(f.pick != null, "a $refs target is read");
  assert(f.pick.name == "out" && f.pick.n == 5, "and named: {f.pick.name}");

  g = Tree.parse(q("<'leaves':[<'name':'x','n':1>,<'name':'y','n':2>],'pick':<'$ref':'#/leaves/1'>>"));
  assert(g.pick.name == "y", "a pointer into the list names its element: {g.pick.name}");
  g.leaves = [];
  g.leaves += [Leaf { name: "new", n: 9 }, Leaf { name: "newer", n: 10 }];
  assert(g.pick == null, "a reference read from a document reads null once its record is removed");
  println("read references: 7 documents held");
}
"#;

#[test]
fn a_pointer_names_a_live_record_of_the_references_type() {
    let dir = std::env::temp_dir().join(format!("loft_json_ref_read_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("scratch");
    fa::write(dir.join("refs.loft"), PROGRAM).expect("program");
    let mut backends = vec!["--interpret"];
    if loft::platform::c_runtime_posix_host() {
        backends.push("--native-c");
    }
    for backend in backends {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .args([backend, "--no-warnings", "refs.loft"])
            .current_dir(&dir)
            .env("LOFT_TIMEOUT", "300")
            .output()
            .expect("loft runs");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim_end(),
            "read references: 7 documents held",
            "{backend}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let _ = fa::remove_dir_all(&dir);
}
