<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# DEPS_INVENTORY.md — the `Vec<u16>` dep-list: meanings, sites, migration design

[STABILITY_HOTSPOTS.md](STABILITY_HOTSPOTS.md) **H2 step 1** (2026-06-11).
The dep vector carried by `Type::Text/Reference/Vector/Enum/Sorted/Hash/
Index/Spatial/Function` encodes at least FIVE distinct meanings; readers
must know which one the writer intended and nothing checks it.  This
inventory fixes the semantic model in writing, classifies every reader /
writer / converter, records what a corpus probe found, and designs the
step-2 migration.  Conceptual background: [LIFETIME.md](LIFETIME.md).

## The two address spaces

A dep entry is an index — into WHAT depends on where the `Type` lives:

| Space | Lives in | Entry means |
|---|---|---|
| **Frame space** | a `Function` variable table entry (`vars.tp(v)`), any parse-time expression result | caller frame VARIABLE number — "this value borrows from var N's store" |
| **Def space** | `Definition.returned`, `Definition.attributes[].typedef` | callee ATTRIBUTE index — "the result borrows from parameter N" |

A `Type` value silently changes space when it crosses a definition
boundary.  The TWO legitimate converters:

- **def → frame**: `parser/mod.rs::resolve_deps` (via `call_dependencies`)
  — at a call site, maps the callee's attr indices through the actual
  argument types into caller var numbers.  `filter_hidden` (same fn)
  strips hidden-buffer attrs for Reference returns first.
- **frame → def**: `ref_return` (`parser/control.rs`) — promotion maps a
  returned local to an attr index through `attr_names`; only attr
  indices are pushed into the def-space dep list (#306 made
  transitively-reached vars merge-only for exactly this reason).

Every OTHER place that lets the two spaces touch is listed under
*Crossing sites* below.

## The marker values (overloads beyond indexing)

| Marker | Space | Meaning | Home |
|---|---|---|---|
| `deps.contains(&u16::MAX)` on a struct FIELD `Reference` | def space | #328 POINTER field (`reference<T>` — 12-byte DbRef, not inline bytes) | written `parser/definitions.rs:1636`; read `data.rs::has_value_cycle`, `parser/mod.rs:3005` (`type_carries_closure` stops at it), `parser/objects.rs:1862`, `compile.rs:418` |
| `u16::MAX` dep on a closure auto-Reference | frame space | "share-marker for a not-yet-known OUTER var" (`vectors.rs:998` — phase-03 refines it to the real var nr) | `parser/vectors.rs:631/870/1268` |
| `dep == [var]` (self-dep) on a keyed local | frame space | @P302 OWNERSHIP marker — "owns its store; `s = []` re-inits in place" | read `generation/dispatch.rs:558`, `generation/dispatch.rs:555` |
| `vec![vr]` on a hidden-buffer arg type | frame space | the work-ref carries ITSELF as dep so the `dep.is_empty()` owned-gate skips it | written `parser/mod.rs::add_defaults` (4183/4199/4224), read `scopes.rs:1636` region |
| `deps.is_empty()` | both | OWNED (no borrow) — the most load-bearing convention in the codebase | everywhere (`is_heap_owned`, `owned_ref` in codegen, …) |
| `CALLEE_FRAME_BIT` (0x8000) on a def-space entry | def space, value = frame var | step 5: a CALLEE-INTERNAL frame-var note (the closure work var a returned fn-ref carries) — skipped by callers, decoded inside the defining fn | written `Deps::callee_frame1` (sole site: `parser/vectors.rs` lambda propagation); decoded `Deps::entries` / `DepEntry::decode` (scopes' declared-return + fn-ref reads, `check_ref_leaks`) |

Note the SAME `u16::MAX` value means two different things (pointer field
vs closure share-sentinel) in two spaces — they never collide today only
because one lives on struct-field typedefs and the other on lambda-local
types.

## Site catalog

### Writers — `depending(on)` (29 sites): ALL frame space

Every `on` argument at every site is a caller var number (spot-checked:
`parser/fields.rs` field-access chains, `parser/objects.rs:134/268`
`closure_param`, `parser/vectors.rs`/`collections.rs` `vec_var`/
`vec_copy_var`/`mat_var`/`db`, `parser/operators.rs:646/1483`,
`parser/control.rs:533/3147`, `variables/mod.rs:795/838`).
`Type::depending` itself (`data.rs:945`) asserts `on != u16::MAX` — the
share-marker is deliberately NOT writable through this path.  ✅ uniform.

### Readers — `.depend()` (55 sites)

| Class | Sites | Space |
|---|---|---|
| `vars.tp(v)` / `function.tp(v)` / parse-time expression types | scopes.rs 311, 909, 958, 1054, 1634(rhs), 1746(lhs), 1832, 2388, 3869, 3901, 3938, 4198, 4212 · parser/* (objects 804, operators 643/1482, expressions 412/429/458/1374/1491, vectors 1371–1857, fields 238/366/372/514, collections 3850, control 532/3359/3504, mod 6278/6281) · variables/mod 808/1142 · state/codegen 1587 | frame ✅ |
| `def.returned().depend()` | dispatch.rs 129 (`is_empty` only — space-blind ✅) · state/codegen 1651, 2343 (index-guarded, see below) · scopes 1631, 1746(rhs), 2379 | def ⚠ see crossings |
| recursive plumbing | data.rs 1030/1035 (RefVar/Tuple union) | inherits |

### Crossing sites (the #306 class — each is a manual space bridge)

1. **`scopes.rs` (`get_free_vars`' declared-return read — the old
   `dep_has_var`)** — RETIRED in step 5 (2026-06-12): the read decodes
   per entry (`Deps::entries`: attr index → name-mapped frame var;
   tagged callee-frame note → direct compare); the positional fallback
   is deleted.  The BLOCK-RESULT (`tp`) dep read is dropped entirely.
   The debug `tp_alone` sentinel that guarded the claim is now ALSO
   removed (cd9c1f94 line): it was NOT "never decides alone" — seven
   corpus scripts (450, 508, repro_p365, four 85-store-lifetime-*) fire
   it — but every firing is a FALSE positive of the retired POSITIONAL
   decode on a field / enum-arm return that COPIES its source into the
   caller's retbuf (`return c.pts`, `match e { Filled{items} => items }`),
   so freeing the local source is CORRECT; re-adding the read would
   suppress that free and LEAK.  Verified value + leak + LOFT_POISON + the
   DA store-free asserts, both backends.  (History: the step-3 bisect
   showed the fallback was load-bearing for factories — `26-closures`'
   two `make_adder` results shared one record without it.)
2. **`scopes.rs:2379–2391` (`check_ref_leaks`)** — pools
   `ret_type.depend()` (def space) and `function.tp(ret_var).depend()`
   (frame space) into ONE `HashSet<u16>` matched against frame var
   numbers.  Debug-only gate, so a def-space entry colliding with a var
   number merely suppresses a leak report — but it is the textbook
   mixed-space container.
3. **`state/codegen.rs:1651 / 2343` (`is_borrowed_view`)** — reads
   def-space deps as indices ✅ but carries
   `(a as usize) >= def.attributes().len()` arms that treat OUT-OF-RANGE
   entries as "borrowed view".  An out-of-range entry would suppress the
   `0x8000` free-source bit → one leaked store per call.
4. **`parser/expressions.rs:1590`** — "Strip ALL deps" before a
   `stores.allocations[u16::MAX]` lookup can fire: a reader defending
   against the share-marker reaching an allocation path.
5. **The TYPE HINT channel (`Parser::expected`)** — CLOSED 2026-07-29
   (loft#666).  `parse_block_inner` installs the block's expected result
   type as the enum-resolution context, and inside a function body that
   is `Definition.returned` — DEF space.  `parse_vector` then rebuilt its
   destination's type with `Deps::frame(parent_tp.depend())`, so an
   attribute index became a caller frame var: in a recursive `-> E`
   function attr 1 became var 1, the retired `__retbuf` placeholder, and
   `scopes::check` null-inited and freed a slot nothing had written
   (`#306` ×2, `#405`, then SIGSEGV).  Only recursion tripped it — a
   non-recursive callee's return carries no attr dep.  A hint answers
   "what SHAPE is expected here", never "what does this borrow", so the
   three hint accessors (`enum_hint` / `vector_hint` / `read_target_type`)
   now return `Type::without_deps()`.  This was NOT a space bridge anyone
   had written on purpose, which is why it was missing from this list:
   the deps rode along on a type copied for an unrelated reason.  Worth
   re-reading the writer catalog with that shape in mind — a `Type` moved
   between spaces for its SHAPE is a crossing site even when no dep code
   is involved.
6. **`parser/mod.rs::fnref_result_type` (the fn-ref call's return)** — CLOSED
   2026-09-04 (loft#1335).  The bridge for a fn-ref call's return type,
   written as a list of the shapes it bridges — text, vector, record, record
   enum — with `other => other` beneath it.  A KEYED collection, a return
   under `?` and a tuple went through untouched, attribute indices intact,
   and the `if` join unioned attribute 0 into a frame-space list: the nightly
   debug-assertions gate's "dep-space violation", nightly since the loft#1245
   guard landed.  Now routed through `Type::borrow_deps` /
   `Type::rewrap_deps`, the one home for "which shapes carry a list", and
   lists nothing — the same cure as the DbRef set (IMPLEMENTATIONS.md).

### Corpus probe (2026-06-11)

An env-gated probe in `scopes::check` scanned every parsed function for
def-space dep entries `>= attributes.len()` across: the full stdlib,
`tests/scripts/100-enhancements.loft`, the moros_glb example, and the
whole crawler kernel self-test (combat @ 458adcc).
**Zero hits** — for `Definition.returned` lists.  CAUTION (learned in
step 3): the probe covered RETURNED types only; BLOCK-RESULT types
(`dep_has_var`'s other input) are mixed-space by contract and out-of-range
entries there are real frame vars, not contamination.  `is_borrowed_view`
reads returned-only, so its `>= len` arms stay defensive (debug-screams
added); `dep_has_var`'s fallback is load-bearing and stays.

## Step-2 migration design (M–L; own quiet window)

Mechanical surface: ~1,000 `Type::X(..)` construction/match sites — too
wide to combine with other work.

1. **Newtype, not enum**: `pub struct Deps(Vec<u16>)` in `src/data.rs`.
   A two-variant enum (frame/def) would force `Type` generics or runtime
   tags through `PartialEq` — the space is positional (where the Type
   lives), so the TYPE SYSTEM hook is the constructor/query surface, not
   the representation:
   - constructors: `Deps::owned()`, `Deps::frame(vars: Vec<u16>)`,
     `Deps::attr_indices(idx: Vec<u16>)`, `Deps::pointer_marker()` (#328),
     `Deps::share_sentinel()` (closure case), `Deps::owned_self(v)`
     (@P302), `Deps::self_carrying(vr)` (hidden buffer).
   - queries: `is_owned()`, `is_pointer_marker()`, `is_owned_self(v)`,
     `frame_vars()` / `attr_indices()` (debug-asserting the space tag).
   - `#[cfg(debug_assertions)] space: DepSpace` field, EXCLUDED from
     `PartialEq`/`Hash` (manual impls), so a cross-space read panics in
     debug and costs nothing in release.
2. **Conversion chokepoints**: `resolve_deps` consumes
   `attr_indices()` + produces `frame(...)`; `ref_return` the reverse.
   `depending()` keeps its `on != u16::MAX` assert and becomes
   frame-only (`debug_assert` the space).
3. **Kill the fossils**: `dep_has_var`'s `a == v` fallback and the
   `>= len` arms in `is_borrowed_view` become unrepresentable; replace
   with the typed query + a debug assert.  `check_ref_leaks` keeps two
   sets.
4. **Mechanics**: migrate one Type variant at a time is NOT possible
   (shared `depend()` plumbing) — instead introduce `Deps` as a type
   alias `pub type Deps = Vec<u16>` first (zero-risk rename commit),
   then flip the alias to the newtype in one commit with the constructor
   sweep.  `make fill` regenerates `fill.rs` if templates name deps.
5. **Validation**: full suite both backends; the #306/#328/#330
   regression files; `tests/leak.rs`; the wrap leak gate; one debug-mode
   full-suite run (the space asserts only live there).

## Status

- [x] Step 1 — this inventory (semantic model, 84+ classified sites,
      corpus probe: no contamination).
- [x] Step 2 — alias-rename commit.
- [x] Step 3 — newtype flip + constructor sweep (named constructors at
      every creation site; `resolve_deps`/`ref_return` typed as THE
      converters; debug screams on contaminated reads).  The dep_has_var
      "fossil" turned out load-bearing (see crossing sites) — kept.
- [x] Step 4 — release + debug full-suite runs green (space asserts live
      in debug).
- [x] Step 5 (2026-06-12) — the positional contract is RETIRED.  What the
      probes showed (debug-tag instrumentation over scripts + docs +
      examples + tools + lib corpora): (a) the in-range attr mapping is
      LOAD-BEARING (77+ non-identity mappings — params are not frame-
      numbered by attr position); (b) no single list ever mixes spaces —
      the mixing is per-PROVENANCE (`def.returned` of a closure factory
      carries the work var, written by `parser/vectors.rs`'s lambda
      propagation; everything else is uniform); (c) the debug-only space
      tag does NOT survive the IR codec (293 `Unknown` tags from cache
      round-trips), so the cure had to be a VALUE tag.  The fix:
      `Deps::CALLEE_FRAME_BIT` (0x8000) marks a callee-internal frame-var
      note inside a def-space list — written by `Deps::callee_frame1`
      (sole writer: the vectors.rs lambda propagation), decoded by
      `Deps::entries()` / `DepEntry::decode` (no def has 0x8000 attrs; the
      constructor rejects var 0x7FFF so `u16::MAX` markers stay
      unambiguous; the bit survives codecs and the build-signature-pinned
      cache makes stale untagged bundles a clean miss).  Readers updated:
      `get_free_vars`' declared-return decode (the old `dep_has_var`
      closure — per-entry match, positional arm deleted), the fn-ref
      `in_ret` check (raw `contains` never matched the tagged value — the
      explicit-`return adder;` cell has an empty block-result list and
      relied on it), and `check_ref_leaks` (pools the decoded frame var
      instead of silently dropping it — this was the false-leak report on
      `___clos_1`).  The BLOCK-RESULT dep read in `get_free_vars` is
      DROPPED: the declared-return + returned-var + return-source-backing
      checks subsume every TRUE return source.  The debug `tp_alone`
      sentinel that guarded this is REMOVED (its firings — seven scripts —
      are all false positives of the retired positional decode on field /
      enum-arm returns that copy into the retbuf; freeing the source is
      correct, and re-adding the read would leak it).
      Regression: `loft-suite/attic/scripts/297-closure-factory-explicit-return.loft`
      (both backends).  Residuals found en route, NOT step-5 scope:
      armed-lib-debug builds (`profile.dev.package.loft`
      debug-assertions=true) are globally red at baseline
      (`check_ref_leaks` false positives on plain `File` locals, then a
      freed-store use inside `ir_store::write_definition`) — the override
      that ships them off is load-bearing; and the @PLN55 growth assert
      tripped on `n_map_from_json` / `n_glb_pos_min` (lib fns) in armed
      builds — FIXED 2026-06-12: the assert's claim was too broad
      (pass-1 multi-return-site growth is sound and pass-stable; only
      PASS-2 growth is dangerous, and the assert now guards exactly
      that).  See STABILITY_HOTSPOTS § H1 retired note;
      `loft-suite/attic/scripts/298-multi-return-site-ref-buffer.loft`.
