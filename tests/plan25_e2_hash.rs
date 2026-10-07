// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN25 E2 / A3 — a `hash<S[k]>` whose element shares records with a sibling
//! `vector<S>` (the "two views, one record set" pattern). Gate-on, both views
//! are rewritten to the synthetic `__nullable<S>` enum, so the key fields live
//! in the `Some` payload, not at the enum's top level.
//!
//! These guard the key-resolution + lookup rungs that are FIXED on the
//! interpreter:
//!  - rung 1: `key_owner` resolves key field numbers/positions through `Some`
//!    (`Stores::hash`, `create_key`, `determine_keys`, `field_content`);
//!  - rung 2: `get_keys` (the fifth `key_owner` site) returns the key TYPES so
//!    `read_key` pops the right bytes — otherwise the stack misaligns and the
//!    hash-container ref reads as junk (`hash::find` panics on a bad store_nr);
//!  - rung 3: `index_type` keeps the synth-`__nullable<S>` lookup result as
//!    `Type::Enum(_, true)` (like a vector element) so `lookup[k].field`
//!    auto-unwraps through `Some` instead of erroring `Unknown field`.
//!
//! Both backends: the `--native` codegen of a `hash<__nullable<S>>` is fixed
//! (rung 4 — the synth enum's `Some`/`Null` variant structures are built
//! up-front so their type-ids precede the consuming struct, matching the order
//! the native emitter replays; plus `__nullable<S>`-as-bool coerces to its
//! present-check). `--native` runs are skipped when `rustc` is unavailable.
//!
//! `LOFT_NO_CACHE=1` is mandatory: the warm program cache keys on source, NOT
//! on `LOFT_E2_SYNTH`, so a cached gate-off bundle would mask the rewrite.

use loft::file_access as fa;
use std::path::Path;
use std::process::Command;

/// Run the loft binary on `prog`, gate-on, cache-off, with extra `args`.
fn run(args: &[&str], prog: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(args)
        .arg(prog)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_E2_SYNTH", "1")
        .output()
        .expect("spawn loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn rustc_available() -> bool {
    Command::new("rustc").arg("--version").output().is_ok()
}

/// Assert `want` appears in stdout on the interpreter AND (when rustc is
/// present) on `--native` — the keyed-collection-over-nullable path must agree
/// across backends.
fn assert_both(prog: &Path, want: &str) {
    let (ok, out) = run(&["--interpret"], prog);
    assert!(ok, "interpret run failed; stdout={out:?}");
    assert!(
        out.contains(want),
        "interpret missing {want:?}; got {out:?}"
    );
    if rustc_available() {
        let (ok, out) = run(&["--native"], prog);
        assert!(ok, "native run failed; stdout={out:?}");
        assert!(out.contains(want), "native missing {want:?}; got {out:?}");
    }
}

fn probe(name: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("loft_e2_hash_probe");
    fa::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(format!("{name}.loft"));
    fa::write(&path, body).expect("write probe");
    path
}

const SRC: &str = "\
struct Count { t: text, v: integer }
struct Counting { entries: vector<Count?>, lookup: hash<Count[t]> }
fn fill(c: Counting) {
  c.entries = [ Count{t: \"One\", v: 1}, Count{t: \"Two\", v: 2}, Count{t: \"Three\", v: 3} ];
}
fn main() {
  c = Counting { };
  fill(c);
  print(\"len={len(c.entries)} \");
  print(\"three={c.lookup[\\\"Three\\\"].v} one={c.lookup[\\\"One\\\"].v} \");
  print(\"missing={c.lookup[\\\"Five\\\"] == null}\\n\");
}
";

#[test]
fn hash_over_nullable_vector_lookup_and_field_access() {
    // len preserved; key extraction finds the shared Some-wrapped records by
    // the `t` field through the Some payload; field access unwraps to S.
    assert_both(&probe("counting", SRC), "len=3 three=3 one=1 missing=true");
}

const SRC_ANON_FORLOOP: &str = "\
struct Count { t: text, v: integer }
struct Counting { entries: vector<Count?>, lookup: hash<Count[t]> }
fn fill(c: Counting) {
  c.entries = [ {t:\"One\",v:1}, {t:\"Two\",v:2}, {t:\"Three\",v:3}, {t:\"Four\",v:4} ]
}
fn main() {
  c = Counting { };
  fill(c);
  add = 0;
  for item in c.entries { add += item.v; }
  print(\"add={add} three={c.lookup[\\\"Three\\\"].v}\\n\");
}
";

#[test]
fn anon_literal_into_hash_field_and_forloop_over_shared_array() {
    // Rung 5: anon `{ … }` literals into a hash field (the element resolves
    // against the `Some` variant, not the enum). For-loop deref: iterating the
    // struct-field nullable vector — stored as a LINKED array of ref slots that
    // shares records with the hash — must DEREF each element (OpVectorRefNullable)
    // before reading `item.v`, not read the rec-id slot as the record inline.
    assert_both(&probe("anon_forloop", SRC_ANON_FORLOOP), "add=10 three=3");
}

#[test]
fn lone_nullable_hash_field_constructs_and_misses_cleanly() {
    // A lone hash field on a fresh struct (no insert): the missing-key lookup
    // must return null, not panic on a misread container ref (rung 2).
    let path = probe(
        "lone",
        "struct Count { t: text, v: integer }\n\
         struct Box { lookup: hash<Count[t]> }\n\
         fn main() {\n  \
           c = Box { };\n  \
           print(\"missing={c.lookup[\\\"x\\\"] == null}\\n\");\n\
         }\n",
    );
    assert_both(&path, "missing=true");
}

const SRC_KEYED_SET: &str = "\
struct Pair { q: integer, v: integer }
struct Bag { items: hash<Pair[q]> }
fn main() {
  bag = Bag { };
  bag.items[1] = Pair{q: 1, v: 10};
  bag.items[2] = Pair{q: 2, v: 20};
  bag.items[3] = Pair{q: 3, v: 30};
  s = 0;
  for e in bag.items { s += e.v; }
  print(\"sum={s} l1={bag.items[1].v} l3={bag.items[3].v}\\n\");
}
";

#[test]
fn keyed_set_into_nullable_hash_inserts_iterates_and_looks_up() {
    // `hash[k] = S{…}` keyed-SET into a nullable hash field: gate-on the element
    // type is `Enum(__nullable<S>, true)`, not `Reference(S)`, so the @P305
    // insert-or-replace routing in `towards_set` must accept the synth-nullable
    // enum too — else the set falls through to the UPDATE-only `OpCopyRecord`,
    // which no-ops on the insert-miss (empty hash), every lookup returns null,
    // and iterating the empty index segfaults. With the routing fixed the set
    // reaches `OpSetKeyed` (find-or-insert; `set_keyed` reads the key via the
    // same `key_owner`-resolved descriptors the lookup uses). The for-loop then
    // needs `for_type`'s Hash arm to keep the element as `Enum(.., true)` so
    // `e.v` unwraps through `Some`.
    assert_both(&probe("keyed_set", SRC_KEYED_SET), "sum=60 l1=10 l3=30");
}

const SRC_LOCAL_KEYED: &str = "\
struct Ent { ck: integer not null, v: integer not null }
fn mk() -> hash<Ent[ck]> {
  r: hash<Ent[ck]> = [];
  r += [Ent{ck: 1, v: 10}];
  r += [Ent{ck: 2, v: 20}];
  r
}
fn main() {
  h = mk();
  s = 0;
  for e in h { s += e.v; }
  print(\"len={len(h)} sum={s} l1={h[1].v} l2={h[2].v}\\n\");
}
";

#[test]
fn typed_local_keyed_append_builds_some_in_place() {
    // `r: hash<S[k]> = []; r += [S{…}]` on a typed LOCAL (not a struct field):
    // gate-on the element type is `Enum(__nullable<S>, true)`, but the keyed
    // element ref `elm` is typed `Reference(Some)`, so the parent type reaching
    // the transparent-construction site (objects.rs) arrives as `Reference(syn)`,
    // not `Enum(syn, true)`. Before the fix only the `Enum(.., true)` form was
    // recognised, so the field path built `Some` in place while the typed-local
    // path built a dense `S` into a temp and a raw `OpCopyRecord` mis-laid it
    // into the `Some` record (wrong offsets, no discriminant → null/garbage).
    // Accepting the `Reference(syn)` form too makes both build `Some` in place;
    // the returned hash then iterates and looks up correctly across a deep copy.
    assert_both(
        &probe("local_keyed", SRC_LOCAL_KEYED),
        "len=2 sum=30 l1=10 l2=20",
    );
}

#[test]
fn method_on_nullable_vector_element_native() {
    // A struct method (`fn val(self: P)`) registers `val` as a Routine ATTRIBUTE
    // on `P`. When `P` is nullable-wrapped (`vector<__nullable<P>>`),
    // `nullable_enum_for` must NOT copy that method attribute into the `Some`
    // variant — else `--native` emits a `db.field(P, "val", t<N>)` for a type it
    // never declares (E0425). Calling the method through the nullable element
    // must work on both backends.
    let path = probe(
        "method",
        "struct P { x: integer }\n\
         fn val(self: P) -> integer { self.x * 10 }\n\
         fn main() {\n  \
           ps = [ P{x:1}, P{x:2} ];\n  \
           print(\"v0={ps[0].val()} v1={ps[1].val()}\\n\");\n\
         }\n",
    );
    assert_both(&path, "v0=10 v1=20");
}

#[test]
fn null_in_shared_vector_is_not_indexed_by_the_hash() {
    // @PLN25 Scope B (design point 3): a `Null` element in a shared nullable vector
    // STAYS in the vector (counted by `len`) but is NOT indexed by the sibling hash —
    // so it is unreachable by key and cannot collide with a real empty-key record.
    // `len(entries)=3` (incl. the null), `len(lookup)=2` (only the two `Some` records).
    let path = probe(
        "null_skip",
        "struct Count { t: text, v: integer }\n\
         struct Counting { entries: vector<Count?>, lookup: hash<Count[t]> }\n\
         fn main() {\n  \
           c = Counting { };\n  \
           c.entries = [ Count{t:\"One\",v:1}, null, Count{t:\"Three\",v:3} ];\n  \
           print(\"veclen={len(c.entries)} hashlen={len(c.lookup)} \");\n  \
           print(\"one={c.lookup[\\\"One\\\"].v} three={c.lookup[\\\"Three\\\"].v} \");\n  \
           print(\"missnull={c.lookup[\\\"Two\\\"] == null}\\n\");\n\
         }\n",
    );
    assert_both(&path, "veclen=3 hashlen=2 one=1 three=3 missnull=true");
}
