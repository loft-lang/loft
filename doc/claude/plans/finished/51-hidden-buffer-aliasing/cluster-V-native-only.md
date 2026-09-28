# Cluster V — Native-only failures

**Severity (split by failure mode, per sub-cluster):**

| Sub-cluster | Corruption / panic / hang | Leak | Status |
|---|---|---|---|
| V-a (tuple schema mismatch — probes 29, 41, 44, 45, 48, 50) | Native produced silently or loudly wrong values (Canvas2.data=null, integer truncation, field reordering) | NONE | ✅ FIXED (commit `b69a1707`) |
| V-b (nested tuple codegen — probe 40) | Native rustc rejected with E0605 (`DbRef` cast to `(DbRef, DbRef)`) | Canvas×24/iter still present on `--interpret` | ✅ Corruption FIXED (commit `92ebe8dc`); leak remains (not graduated) |
| V-c native (lambda dispatch — probes 30, 59, 62) | Native rustc `unreachable!("invalid fn-ref")` | NONE for 59, 62; Canvas×6/iter for 30 (interp-side, separate from V-c) | ✅ FIXED (commit `e4cd328d`) |
| V-c interp (lambda dispatch — probes 30, 59, 62) | Interp corrupted main's stack frame (loop var `i = 65535` after lambda return) | Canvas×6/iter for 30 (separate from corruption) | ✅ Corruption FIXED (commit `5eb7d90d`); leak remains for 30 |

**Status (2026-05-28): ✅ ALL CORRUPTION CLOSED.**  Two probes (30, 40) still carry leaks separate from their corruption fixes; both stayed in `probes/` (substitutes graduated to `tests/scripts/`).  See § Graduated probes for the substitution rationale.

**Graduated probes:**
- V-a: probe 29 → `tests/scripts/147-plan51-cluster5a-tuple-return.loft`
- V-c: probe 53 (lambda-with-captures, clean substitute for probe 30) → `tests/scripts/150-plan51-cluster5c-lambda-with-captures.loft`
- V-b: not graduated (probe 40 still leaks; no clean substitute available — V-b is sole-probe)

**Backend asymmetry:** Opposite from clusters II/III — here NATIVE was the failing side (interp passed for V-a/V-b; interp also failed for V-c-interp).

---

## Verified root cause (probe 29 & family)

(Probe 30's mechanism: see V-c sub-cluster section below.)  12 new probes (40-51) added during the deep dive to map the scope; see § Probe scope sweep.

**V-a mechanism:**

`src/generation/mod.rs:1528` emits `db.structure("{name}", 0)` into the generated native binary for each loft struct type, then adds attributes via subsequent `db.field(...)` calls.  But it **does NOT propagate `Definition::field_groups`** — the `LinkedFieldGroup::Tuple` entries that `tuple_def` (`src/data.rs:2586`) registers for tuple types so the compiler-side `finish_type` can pack tuple elements as one atomic block via `calculate_positions_with_groups`.

Consequence: the compiler's database has tuple positions/size from the **group-aware** layout; the native binary's runtime database (rebuilt at startup by the generated `db.structure` + `db.field` + `db.finish` sequence) has positions/size from the **simple alignment-descending packer** (`calculate_positions`).  The IR is emitted against the compiler-side layout (hardcoded field positions) but executes against the native-side layout (different sizes).

For `(Canvas, Canvas)`:
- Compiler side (with field_groups): positions `[0, 16]`, size **28**, align 8.
- Native runtime side (without field_groups): positions `[0, 12]`, size **24**, align 8.
- IR writes Canvas2 at offset 16 (per compiler).  `OpCopyRecord(_src, dst, tp=66)` at the call site uses `stores.size(66) = 24` (per runtime), copies bytes 0-23 — missing Canvas2.data at 24-27.  Then `copy_claims` walks Parts::Struct using positions [0, 12], reading from src.pos 12 (PADDING in the source's actual layout) and producing a null data pointer.

**Affected probes:** 29 (tuple-return), 30 (lambda-return), 40, 41, 44, 45, 48, 50 (all tuple-shape variants confirmed to fail; see § Probe scope sweep).

## Probe scope sweep (12 probes, 40-51)

After confirming the schema-mismatch mechanism, ran a focused scope sweep.  Trace tools in tree: `LOFT_TRACE_COPY=1` (OpCopyRecord src/dst/size/free_src), `LOFT_TRACE_FINISH=1` (finish_type entry+exit, tuple types only).  Both gated on env vars so they're zero-cost when off.

| # | Probe | Shape | Interp | Native | Why |
|---|---|---|---|---|---|
| 29 | tuple-return | `(Canvas, Canvas)` | ✅ | 💥 cb[0]=null(oob) | canonical; Canvas2.data at IR pos 24 > runtime size 24 |
| 40 | nested-tuple-of-canvases | `((C,C),(C,C))` | (parse) | ❌ rustc E0605 | separate native codegen bug: tries to cast DbRef to `(DbRef,DbRef)` |
| 41 | three-canvas-tuple | `(Canvas, Canvas, Canvas)` | ✅ | 💥 cc.w=43 (garbage half-i64) | Canvas3 at IR pos 32+12=44 > runtime size 36 |
| 42 | canvas-then-int (small) | `(Canvas, integer)` (val < 2^32) | ✅ | ✅ PASS BY LUCK | int's lower 4 bytes copied, upper 4 zeroed — invisible when value fits in 32 bits |
| 43 | int-then-canvas | `(integer, Canvas)` | ✅ | ✅ | element_size matches storage size; layouts agree |
| 44 | canvas-canvas-int | `(Canvas, Canvas, integer)` | ✅ | 💥 cb[0] | Canvas2.data still truncated; trailing primitive irrelevant |
| 45 | text-bearing-struct-tuple | `(Named, Named)` with `text` fields | ✅ | 💥 name | text field same shape as vector header (4 bytes); same truncation |
| 46 | flat-struct-tuple | `(Flat, Flat)` (struct of two i64) | ✅ | ✅ | Flat storage size 16 == stack-side 12 vs storage divergence — actually both layouts agree here because Flat's storage size is a multiple of its align |
| 47 | tuple-local-not-return | `t = (a, b); (ca, cb) = t` at local | ✅ (leak ×1) | ✅ | bug is ref_return-promoted-tuple-buffer specific; local tuples take a different codegen path |
| 48 | canvas-bigint | `(Canvas, integer)` (val > 2^32, e.g. 5e9) | ✅ | 💥 k=705032704 | exposes probe 42's silent truncation: lower 32 bits preserved, upper 32 bits zeroed (705032704 = 5e9 mod 2^32) |
| 49 | int-int-tuple | `(integer, integer)` | ✅ | ✅ | both elements same size/align; group-aware and simple-packer layouts identical |
| 50 | text-int-tuple | `(text, integer)` | ✅ | 💥 s='' | simple packer **reorders by alignment-descending**: int gets pos 0, text pos 8.  IR writes text at 0; runtime reads it at 8 (where the int's lower bytes sit) → 0 → empty string |
| 51 | tuple-as-arg | `fn check(pair: (Canvas, Canvas), …)` | ✅ (leak ×1) | ✅ | tuple arg is a native Rust value-tuple `(DbRef, DbRef)` — never serialised through the database; bug only fires on the ref_return path |

**Scope conclusions:**

- **The bug is universal to tuples whose group-aware and simple-packer layouts diverge.**  Probes 29/41/44/45 (≥2 Reference elements) and 50 (mixed-alignment primitives) confirm this.
- **Probe 48 is the most important new finding** — `(Canvas, integer)` is not "safe": small integers pass by luck because the truncated upper bytes happen to be zero, but any value ≥ 2^32 corrupts silently.  Same risk applies to any tuple slot beyond `runtime_size`.
- **Probe 50** widens the bug from "truncation" to "**field reordering**": when the simple packer reorders by alignment-descending, IR and runtime disagree about which field is at which offset.  `(text, integer)` is read with int and text swapped.
- **Probe 46 (Flat, Flat) PASSING** narrows the scope: pure-primitive structs whose total size is a multiple of their alignment don't trip the bug, even though element_size (12) and stack-align (4) differ from storage (16, align 8) — because group-size and simple-packer total both come out to 32 with identical positions.
- **Probe 47 (local) and 51 (arg) PASSING on native** identifies the trigger path precisely.  Both keep the tuple as a **native Rust value-tuple** (`(DbRef, DbRef)`) on the stack — the codegen emits `let var_t: (DbRef, DbRef) = ...; var_ca = var_t.0` for `(a, b) = t`, and arg-side passes the tuple as a Rust function parameter typed `(DbRef, DbRef)`.  Neither path serialises the tuple through the database (no OpDatabase, no OpCopyRecord on the tuple), so the schema mismatch never matters.  Only `ref_return`'s `rewrite_tail_tuple_with_work_ref` (src/parser/control.rs:1024) materialises a tuple as a database record — that's the unique trigger path.  The interp-side `×1 Canvas` leak in these probes is unrelated to the native bug (it's the local-tuple equivalent of Cluster II's hidden-buffer free-pattern issue).

### Trace evidence

`LOFT_TRACE_FINISH=1` on probe 29:

```
[ENTER t_nr=66 name=__tuple<Canvas,Canvas> size_before=65535 groups_before=1]
[finish_type   t_nr=66 size=28 align=8 groups=1]    ← compiler side, CORRECT
[ENTER t_nr=66 size_before=28 groups_before=1]      ← early-return (already finished)
[ENTER t_nr=66 size_before=65535 groups_before=0]   ← native binary init, FRESH structure(), no groups
[finish_type   t_nr=66 size=24 align=8 groups=0]    ← runtime side, WRONG
```

`LOFT_TRACE_COPY=1` on the same run shows the runtime's wrong size in action:

```
[copy] OpCopyRecord src=#4@1,8 dst=#1@1,8 tp=66 size=24 free_src=true
[copy]   after: rec=1 c1_w=4 c1_data=25 c2_w=7 c2_data=0   ← c2_data=0 is the bug
```

The 24-byte copy clips before Canvas2.data (which IR put at offset 24).

---

## Probe 29 — tuple-of-heap-structs return

```loft
fn split(p: P) -> (Canvas, Canvas) {
  a = alloc_canvas(4, 5, p.tag);
  b = alloc_canvas(7, 9, p.tag * 2);
  (a, b)
}

fn main() {
  for i in 0..6 {
    (ca, cb) = split(P { tag: i });
    assert(cb.data[0] == i * 2, ...);
  }
}
```

**Interpret:** PASSES.
**Native:** COMPILES + RUNS, but assertion fires at iter 0: `cb[0] = null(oob)`.

The generated Rust at `/tmp/loft_native_*.rs:775:14` panics on the assertion message itself.  The compiled binary executes, but the SECOND tuple element (`cb`) is empty / corrupted on iter 0.

### Hypothesis

The native codegen for tuple-of-heap-structs has a buffer-passing bug.  Tuples in loft are represented internally as synthetic struct types (`__tuple<Canvas, Canvas>`).  ref_return promotes the tuple-typed local to a hidden buffer.  Native lowering of "tuple containing two heap fields" must:

1. Allocate a tuple record.
2. Allocate Canvas a (first field).
3. Allocate Canvas b (second field).
4. Write a and b into the tuple's fields.

Possible bug: native emits writes for the first field (a) but the second field write is silenced, OR the second field is written to the wrong slot.

Interpret handles this correctly via its `Insert`-style codegen for tuple construction (separate field-init opcodes).

### What to investigate

1. Locate the native codegen for `Value::Tuple(elements)` and synthetic-`__tuple` types in `src/generation/`.
2. Capture the generated Rust file from `/tmp/loft_native_*.rs` BEFORE the binary runs (use `LOFT_KEEP_NATIVE_RS=1` if such a flag exists; otherwise run with build-only).
3. Inspect lines around 775 in the generated file to see how the tuple's two Canvas fields are populated.

---

## Probe 30 — lambda returning heap struct

```loft
fn main() {
  make_renderer = fn(p: P) -> Canvas {
    cv = alloc_canvas(4, 5, p.tag);
    cv
  };
  for i in 0..6 {
    p = P { tag: i };
    cv = make_renderer(p);
    assert(cv.data[0] == i, ...);
  }
}
```

**Interpret:** Assertion fails at `iter 65535: data[0] = null(oob)`.  The `65535` is u16::MAX — the LOOP VARIABLE `i` itself has been corrupted to the null sentinel.

**Native:** Panics in the generated Rust at line 2264.

### Symptom analysis — interpret

When the assertion message reads `iter 65535: data[0] = null(oob)`, the loop variable `i` was overwritten to `65535` somewhere during the iteration.  Lambda execution corrupted main's stack frame.

Lambda values in loft are represented as `(d_nr: u32, closure_capture: DbRef)` 16-byte fn-ref slots.  Calling a lambda likely involves:

1. Loading the fn-ref from main's slot.
2. Setting up the lambda's frame (its captured args + locals).
3. Executing the lambda body.
4. Returning, restoring main's stack pointer.

If the lambda's frame setup oversteps and writes into main's frame's slot (where `i` lives), main's loop variable gets corrupted.

### Symptom analysis — native (verified 2026-05-28)

Captured generated Rust via `LOFT_KEEP_NATIVE_RS=1`.  Line 2264 reads:

```rust
let mut var_cv: DbRef = {
    let _farg_0 = var_p;
    match var_make_renderer.0 {
        _ => unreachable!("invalid fn-ref: {} in make_renderer", var_make_renderer.0)
    }
};
```

**The dispatch match has NO real arms** — only the `_ => unreachable!` fallback.  The native codegen for fn-ref dispatch never emits actual `<lambda_d_nr> => n_<lambda>(...)` arms.  Every fn-ref call panics with the "invalid fn-ref" message regardless of which lambda is being called.

This is unrelated to the schema-mismatch class.  It's a missing-codegen branch in fn-ref dispatch.

### Hypothesis

Lambdas have a distinct codegen path from named function calls.  The fn-ref dispatch + closure-capture mechanism wasn't updated to handle ref_return-promoted hidden buffer args.  Specifically:

- For a named `fn f(p, __hidden) -> T`, the caller pushes `(p, __hidden)` as args and Call(f) does the work.
- For a lambda `make_renderer = fn(p) -> T { ... }`, the call goes through fn-ref dispatch (`OpCallRef` or similar), which might not have the hidden-buffer arg path wired in.

Result: the lambda body uses some stack location that overlaps with the caller's frame, corrupting the caller's `i`.

### What to investigate

1. Read the fn-ref dispatch codegen — `OpCallRef` or `Value::CallRef` handling in both `src/state/codegen.rs` and `src/generation/`.
2. Check whether lambdas engage `ref_return` and `add_defaults` like named fns do.
3. Trace probe 30 with `LOFT_LOG=ref_debug,type_timeline:i` to see when `i` flips to 65535.

---

## Sub-cluster split (post-gap-investigation 2026-05-28)

The 2026-05-28 gap investigation split Cluster V into three orthogonal sub-clusters:

| Sub | Probes | Mechanism | Fix surface |
|---|---|---|---|
| **V-a — Tuple schema mismatch** | 29, 41, 44, 45, 48, 50 | Native codegen at `src/generation/mod.rs:1528` emits `db.structure / db.field` but does NOT propagate `field_groups`.  Runtime's tuple type uses simple-packer layout; IR uses group-aware layout.  Misalignment → truncation (29/41/44/45/48) or field-reorder (50). | Add `pub fn add_tuple_group(&mut self, tp: u16, members: &[u16])` to `Stores`; native codegen emits it after the field-set sequence for tuple types. |
| **V-b — Nested tuple codegen** | 40 | `((C,C),(C,C))` codegen emits `n_pair(...) as (DbRef, DbRef)` — casts a heap-promoted tuple DbRef to a Rust value-tuple.  Type-mixup at the call site. | Codegen needs to recognise when a tuple-typed sub-expression is a heap-promoted DbRef (vs a stack value-tuple) and emit destructuring (`{ let _t = n_pair(...); (_t.0, _t.1) }` or equivalent reads).  Distinct from V-a. |
| **V-c — Lambda dispatch (ref_return shape)** | 30, 59, 62 | Native fn-ref dispatch emits `match var_fnref.0 { _ => unreachable!() }` for lambdas whose return triggers ref_return (struct with nested heap, or bare vector).  Root cause: `src/generation/emit.rs:519-523` candidate-filter excludes only text-RefVar and `__closure` attributes — NOT `Attribute.hidden = true` (the ref_return marker).  So when the lambda's signature has a hidden buffer param, the candidate's `visible_attrs.len()` exceeds the call site's `user_arg_match`, no candidate is collected, no arm is emitted.  Interp side: separate stack-frame corruption mechanism (`iter 65535`). | (1) Extend the filter at `src/generation/emit.rs:519-523` to also exclude `a.hidden`.  (2) Extend the arm-emit loop at `:657-672` to push an appropriate hidden-buffer expression (caller-side work-ref) when the candidate's attribute has `hidden = true` and `Type::Reference / Vector`. |

## Fix surface (V-a — the schema-mismatch class)

The verified root cause is in **one** site: `src/generation/mod.rs::emit_type_creation` (line 1508) emits `db.structure / db.field` but skips `field_groups`.  The compiler-side database has the correct group metadata via `tuple_def → fill_database → typedef.rs:569 (extend(groups))`; the runtime-side database (in the native binary's init function) never receives it.

**Proposed fix:**

1. **New `Stores` method** (`src/database/types.rs`):
   ```rust
   pub fn add_tuple_group(&mut self, tp: u16, members: &[u16]) {
       self.types[tp as usize].field_groups.push(LinkedFieldGroup {
           kind: LinkedFieldKind::Tuple,
           instance: 0,
           field_indices: members.to_vec(),
           alignment: 0,  // recomputed in finish_type from storage widths
           size: 0,       // recomputed in finish_type from storage widths
       });
   }
   ```
   The `alignment` / `size` zeros are safe: `finish_type` rebuilds `groups_descriptor` from member storage widths (types.rs:304-322) and never reads the pre-stored values.

2. **Codegen call site** (`src/generation/mod.rs::emit_type_fields_mode` or right after, before `db.finish()`):  for each definition whose `Definition::field_groups` contains a Tuple entry, emit:
   ```rust
   db.add_tuple_group(t{n}, &[0, 1, ...]);
   ```

3. **No other Stores changes needed.**  Index groups already propagate (the codegen emits `db.index(...)` which internally re-pushes its LinkedFieldGroup).

**Effort:** S (~half day, one method + one codegen emit site + regression test).

**Risk:** very low.  The change only adds metadata the runtime already knows how to use (`calculate_positions_with_groups` is an existing code path).  No new runtime semantics; no IR changes.

## Fix surface (V-b)

Separate sub-cluster — defer to follow-up work.  In the tuple-element codegen path (where a tuple's element is itself a call returning a heap-promoted tuple).  Likely fix: in `output_code_inner` for `Value::Tuple` with element type `Reference(__tuple<…>)`, destructure via field reads instead of `as` casting.

## Fix surface (V-c)

**Verified root cause** (2026-05-28 deep dive via probes 52-62):

The fn-ref candidate-matching filter at `src/generation/emit.rs:519-523` is:

```rust
let visible_attrs: Vec<&crate::data::Attribute> = def
    .attributes
    .iter()
    .filter(|a| {
        !matches!(a.typedef, Type::RefVar(ref inner) if matches!(**inner, Type::Text(_)))
            && a.name != "__closure"
    })
    .collect();
if visible_attrs.len() != user_arg_match {
    continue;  // skip this candidate
}
```

It excludes text work-buffer attrs and the `__closure` attr, but NOT the ref_return-promoted hidden buffer (which loft's parser marks via `Attribute.hidden = true` per src/data.rs:1404).  For a lambda with `fn(p: P) -> Canvas`, the post-ref_return signature is `n___lambda_0(var_p, var_cv: DbRef) -> DbRef` (two attrs).  The call site passes one user arg.  `visible_attrs.len() == 2 != user_arg_match == 1` → `continue` → no candidates → only `_ => unreachable!()` arm.

**Probe scope sweep results (52-62):**

| Probe | Lambda return | Lambda has hidden buf? | Native |
|---|---|---|---|
| 52 | `integer` | No | ✅ |
| 53 | `integer` (+capture) | No | ✅ |
| 54 | `integer` (passed as arg) | No | ✅ |
| 55 | `integer` (in struct field) | No | ✅ |
| 56 | `integer` (multiple lambdas) | No | ✅ |
| 57 | `integer` (immediate invoke) | No | ✅ |
| 58 | `integer` (calls named fn) | No | ✅ |
| 60 | `Flat` (struct, no nested heap) | No (no ref_return) | ✅ |
| 61 | `(integer, integer)` (value-tuple) | No | ✅ |
| 30 | `Canvas` | Yes | 💥 |
| 59 | `vector<integer>` | Yes | 💥 |
| 62 | `Canvas` (+capture) | Yes | 💥 |

The bug fires iff the lambda's body engages `ref_return`'s hidden-buffer promotion.  Captures don't matter (62 vs 30 same failure shape).  Return-type categorisation:
- **Heap struct with nested heap** (Canvas, Named) → ref_return engages → V-c fires.
- **Bare heap collection** (vector<T>, Hash<…>, Sorted<…>) → ref_return engages → V-c fires.
- **Flat heap struct** (no nested heap fields) → ref_return does NOT engage → V-c does NOT fire.
- **Value tuple** (no heap) → returned as Rust tuple → V-c does NOT fire.
- **Primitive** → V-c does NOT fire.

**Proposed fix:**

1. **Extend the filter** at `src/generation/emit.rs:519-523` to also exclude `a.hidden`:
   ```rust
   .filter(|a| {
       !a.hidden  // ref_return / text_return hidden buffer
           && !matches!(a.typedef, Type::RefVar(ref inner) if matches!(**inner, Type::Text(_)))
           && a.name != "__closure"
   })
   ```

2. **Extend the arm-emit loop** at `src/generation/emit.rs:657-672` to push an appropriate hidden-buffer expression when iterating a candidate's attrs:
   ```rust
   for a in &candidate_def.attributes {
       if a.hidden && matches!(a.typedef, Type::Reference(_, _) | Type::Vector(_, _) | ...) {
           // pass a fresh work-ref or the caller's destination buffer
           synthetic.push(Value::RawExpr("/* TBD: hidden-buffer arg */".to_string()));
       } else if matches!(a.typedef, Type::RefVar(ref inner) if matches!(**inner, Type::Text(_))) {
           synthetic.push(Value::RawExpr(work_buf_expr.clone()));
       } else if a.name == "__closure" { ... }
       else { ... }
   }
   ```

   The "TBD" is the design question: what does the caller pass for the hidden buffer?  Probably either (a) a freshly-allocated DbRef of the right type or (b) the destination LHS the dispatch result is being assigned to.  Direct-call emission (in src/generation/dispatch.rs:117-130) uses a `{ let _src = call(...); OpCopyRecord(_src, dst, ...); }` block; the fn-ref-dispatch arm could mirror that pattern.

3. **V-c interp side** (probe 30 / 59 / 62 `iter 65535`): separate diagnosis — the lambda's frame setup writes into the caller's stack slot for the loop variable.  Needs `LOFT_LOG=ref_debug,type_timeline:i` trace to pin the offending opcode.  Distinct bug from the native dispatch arm gap.

## What we know vs. don't

| | Status |
|---|---|
| Probes 29/41/44/45/48/50 fail on native (V-a schema mismatch) | ✅ Verified |
| Probe 30 fails differently on native and interpret (V-c lambda dispatch) | ✅ Verified |
| Probe 40 fails with rustc E0605 (V-b nested tuple codegen) | ✅ Verified |
| Probes 42/43/46/49 PASS on native (no schema divergence) | ✅ Verified |
| Probes 47/51 PASS on native (tuple stays a Rust value-tuple, never serialised) | ✅ Verified |
| **V-a root cause — field_groups not propagated by `src/generation/mod.rs:1528`** | ✅ Verified via dual trace `LOFT_TRACE_FINISH` + `LOFT_TRACE_COPY` |
| **V-a fix surface — `Stores::add_tuple_group` + codegen emit site** | ✅ Designed |
| V-b root cause — DbRef cast to Rust tuple at nested-tuple call site | ✅ Pinned (rustc message points at site) |
| V-c root cause — fn-ref dispatch match has no real arms | ✅ Pinned (generated Rust inspected) |

### Earlier mis-hypothesis (kept for history)

A pre-investigation draft hypothesised that probe 29's bug was an OpFreeRef-after-OpCopyRecord double-free producing dangling vectors from shallow copies.  This was wrong — `OpCopyRecord` does a real deep copy (allocates fresh records for nested vectors via `copy_claims_seq_vector` at `src/database/allocation.rs:827`).  The actual mechanism is the tuple size/positions mismatch documented in § Verified root cause above.

The trace evidence that disproved the shallow-copy theory: `LOFT_TRACE_COPY=1` shows `OpCopyRecord src=#2 dst=#4 tp=65 size=12` correctly deep-copies Canvas, allocating a fresh vector record in dst's store.  The corruption only appears at the OUTER copy (`tp=66 size=24`) which clips before Canvas2's data field — a SIZE problem, not a free-order problem.

## Investigation tasks (open)

For V-c (probe 30):
1. Read `src/generation/` fn-ref dispatch emission (`emit_fn_ref_dispatch` or equivalent) to find why match arms aren't generated for registered lambdas.
2. Separately, trace probe 30 interpret with `LOFT_LOG=ref_debug,type_timeline:i` to pin which opcode flips `i` to 65535.

For V-b (probe 40):
1. Find the codegen site that emits `n_pair(...) as (DbRef, DbRef)`.  Search `src/generation/dispatch.rs` for tuple-element handling.
