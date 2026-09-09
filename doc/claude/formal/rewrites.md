<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Native rewrites — the emitter's cheaper forms, and what each one assumes

**Scope.** The forms the NATIVE emitter (`src/generation/`) substitutes for the
template emission when a side condition holds: a vector header derived once for a
loop, a record scalar read once, a view's header taken at its binding, a stdlib
wrapper emitted as its op, a callee handed the values its caller already holds.
Each is a REWRITE — the program's observable behaviour is unchanged — and the
interpreter applies none of them, which is what makes it the oracle for every cell.
The runtime-side units of the same arc (the per-allocation and per-record
bookkeeping, the append path) are not rewrites and are not here; the IR lowerings
both backends share (the fused scalar append, append in place) belong to the
chapters of the constructs they lower.

**Why a chapter.** Every rewrite rests on ONE invariant, and until now each lived
as prose in its own design section (`doc/claude/plans/157-native-4x-drawing/
DESIGN.md`), with no name a code site could cite.  Measured: `src/generation/
hoist.rs` cited zero rules while enforcing eight, and the first defect of the
family — a user function inlined as a stdlib wrapper (D-rw-1 below) — was the
rule's own qualifier ("stdlib") that the code had never asked.  A rewrite that
COMPOSES others (R-Inputs takes R-Callee's admission, R-Scalar's and R-Header's
candidates, and moves them through a call) needs them as citation targets.

**How a rule is written.** Shape ⟹ emitted form, under a side condition; then the
switch that restores the previous emission and the falsifier that catches a stale
assumption.  A site enforcing a rule cites its `@FR-R-…` tag
([README.md § Rule tags](README.md)).

## Rules

### Every rewrite is switchable and falsifiable

```
  (R-Switch)     a rewrite ships with a GENERATION-TIME switch that emits the previous
                 form, and with a falsifier: a checking form that re-derives what the
                 rewrite assumed and panics when it is stale (LOFT_HOIST_VERIFY=1), or
                 an emission pin where there is nothing to re-derive.  The interpreter
                 applies no rewrite and is the oracle of every cell.
```

**In words.** The switch is the before-half of an A/B on one binary and the first
bisect step for a native-only wrong answer; the falsifier is what makes "the values
agree" a proof rather than a coincidence — a header-served read and a runtime read
of an unmoved vector answer the same number, so agreement alone cannot tell a sound
rewrite from a lucky one.  `PERFORMANCE.md § Design: P2` and `NATIVE.md` list the
switches; `hoist_verify` in `Output` is the one flag every checking form reads.

### A vector reached by a pure path has one header for a loop that cannot move it

```
  (R-Header)     in a loop body that writes no store, a vector reached by a PURE
                 PATH P — a variable, or const-offset fields over one — has one
                 header (store, record, length) for the whole loop: the emitter
                 derives it once before the loop, and every element read, element
                 write and len(P) in the body uses it.  A rebind of P's root inside
                 the body removes P; a nested loop's prelude skips a path an outer
                 one already holds.
```

**In words.** loft#885 stage 1 (the header) and stage 2 (the element read fused onto
it), with @PLN157 P4d's path key (`lay.px` is `(lay, [32])`) and the hoisted loop
bound (`len(P)` served by the header, queue item 1).  The path is pure, so evaluating
it once in the prelude is exactly the evaluation the body repeats.  Switches
`LOFT_NO_VECTOR_HOIST` (no header at all) and `LOFT_NO_ELEM_FUSE` (the header, but
the element read unfused); falsifier `LOFT_HOIST_VERIFY=1`, whose checking form
re-derives the header at every access.  Sites: `hoist::vector_path`,
`hoist::vector_candidates`, `Output::begin_vector_hoist`, the registry's
`FusedElementReadEmitter`, `FusedElementWriteEmitter` and `HoistedLengthEmitter`.

### An in-place scalar set disturbs no header

```
  (R-InPlace)    a fixed-width scalar set through an EXISTING address — an element,
                 a record field, a local — moves no record and changes no length,
                 so it cannot invalidate a header: a loop whose store writes are all
                 such sets keeps (R-Header).  What may run in such a loop is an
                 ALLOW-LIST (the store-free ops, IN_PLACE_SET_OPS); an op missing from
                 it costs the rewrite, never correctness.
```

**In words.** @PLN157 P4a.  Aliasing is free for headers under this rule — an
in-place write moves nothing, so no header, aliased or not, can go stale; that is
why the header needs no write set while a scalar (R-Scalar) does.  Switch
`LOFT_NO_WRITE_HOIST`.  Sites: `hoist::IN_PLACE_SET_OPS`,
`hoist::blocks_header_hoist`.

### A callee is admitted by what its body writes, one call deep

```
  (R-Callee)     a user callee whose store writes are all (R-InPlace) sets — through
                 any address, transitively through callees it admits the same way —
                 or whose only writes land fixed-width scalars in its own hidden
                 return buffer, is admitted under (R-InPlace) as a direct set is.  A
                 CallRef, a Parallel, a Yield and a recursion keep the WRITER
                 verdict.
```

**In words.** @PLN157 § V-l (`in_place_only_writer`) and § V-c
(`retbuf_only_writer`): the P4a argument applied to a whole body.  The arguments
still walk below the call node, so a growing op inside one blocks on its own.
Switches `LOFT_NO_INPLACE_CALLEE_HOIST`, `LOFT_NO_RETBUF_HOIST`.  Sites:
`hoist::in_place_only_writer`, `hoist::retbuf_only_writer`,
`hoist::call_writes_store`.

### A record scalar the body cannot write is read once

```
  (R-Scalar)     under the (R-InPlace) gate, a scalar field read v.f — v a plain-
                 struct record variable the body does not rebind — holds one value
                 for the whole loop unless the body's WRITE SET reaches
                 (type(v), f): every (record type, offset) its in-place sets target,
                 what an admitted callee writes (R-Callee) — an in-place-only
                 writer's own set, a return-buffer writer's record type WHOLE —
                 and the whole type of any record the body frees.  A write the walk
                 cannot type empties the loop's scalar candidates.  Keys carry the
                 TYPE, never the variable, because aliasing is decided by what a
                 write can reach.
```

**In words.** @PLN157 P4c.  The getter's own `#rust` template is emitted once in
the prelude — the `rec == 0` sentinel included — so the local holds exactly what a
per-iteration read would.  A `vector<integer>` element written at offset 0 reaches
no record's field; a `&`-bound alias, an element view and a callee's parameter all
write the same `(Lay, 0)`.  Switch `LOFT_NO_SCALAR_HOIST`; falsifier
`LOFT_HOIST_VERIFY=1` (`hoisted_scalar_verify` re-reads and compares).  Sites:
`hoist::hoistable`, `hoist::body_writes`, `hoist::callee_writes`,
`hoist::WriteSet::evicts`, `hoist::setter_target`, the registry's
`emit_hoisted_scalar_or_default`.

### A view's header is taken at its binding

```
  (R-View)       a view d = &P bound from a pure path P fixes d's DbRef, so d's
                 header may be derived once, right after the binding, and serve
                 every d[i] and len(d) in the rest of the block — when the remainder
                 passes (R-InPlace), never rebinds d, and indexes d at least once.
                 A rebind of P's ROOT afterwards does not matter: d keeps the DbRef
                 it was given.
```

**In words.** @PLN157 § V-n: the loop hoist's promise applied to the statements
after a `Set(d, P)` instead of a loop body.  A length read alone earns no header (it
is cheaper through the runtime).  Switch `LOFT_NO_VIEW_HOIST`; falsifier
`LOFT_HOIST_VERIFY=1`.  Sites: `hoist::view_def_header`, `Output::bind_view_header`.

### A stdlib one-op wrapper is its op

```
  (R-Wrapper)    a call to a STDLIB function whose whole body is one native op over
                 its own parameters and constants, with leaf arguments (a variable,
                 a literal) and no text operand or result, is emitted as that op
                 with the arguments in the operand positions.  A USER function's
                 call is observable — the live tier may flip it to the interpreter,
                 and its frame is on the shadow call stack — and stays a call.
```

**In words.** @PLN157 § V-o.  The wrapper's compiled Rust body IS the op's template,
so for a frameless, unflippable stdlib method the op is the call; `len(d)` after a
view binding then reads the header.  Leaf arguments only, because the op's operands
are emitted from a fresh list the pre-evaluation map (keyed on node addresses)
cannot see.  Switch `LOFT_NO_WRAPPER_INLINE`; pin `tests/wrapper_op.rs`.  Sites:
`hoist::one_op_wrapper`, `Output::wrapper_op`.

### A callee's invariant inputs cross the call

```
  (R-Inputs)     a callee admitted by (R-Callee), of a plain-struct parameter p it
                 never rebinds, has INVARIANT INPUTS: the scalar fields p.f its own
                 write set does not reach, and the vector paths p.g it views (R-View)
                 or indexes (R-Header).  A caller loop that holds, for the leaf
                 argument variable c it passes for p, the value of (c, f) under
                 (R-Scalar) and the header of (c, g) under (R-Header) — EVERY input
                 of the callee — calls the callee's TWIN: the same body emitted with
                 those values as extra parameters, read in place of the record.  A
                 call missing any input keeps the plain form; the twin exists beside
                 the original, never instead of it.
```

**In words.** @PLN157 § V-p.  The pixel methods read `self.width` / `self.height`
and view `self.data` per call — single reads, so (R-Scalar) cannot hoist them
inside the callee, and the caller's hoisted values do not cross the call.  Handing
them over is sound for exactly the reason each was hoisted: the caller's loop proved
them invariant across the call (its write set includes what the callee writes), and
the callee's own write set proves it does not write them either.  Measured before
the code by hand-editing the emitted Rust: the twin and a full inline of the
callee cost the same (`composite` 4.2× → 2.33× of Rust), because rustc inlines the
small twin — so the rewrite passes values and never clones IR; shipped, the consumer's
`composite` row measures 2.34× (446k → 237k ns/op), `render_lock` / `render_marks` −8 %.  Switch
`LOFT_NO_CALLEE_INPUTS`; falsifier `LOFT_HOIST_VERIFY=1`, which inside the twin
re-reads every input against the record.  Sites: `hoist::callee_inputs`,
`hoist::hoistable` (the mapping through the argument), the twin's emission in
`Output::output_function`, the twin call in `Output::user_fn_call_body`.

### A leaf carries no frame

```
  (R-Leaf)       a function whose body calls no user function and no fn-ref — a LEAF
                 — is emitted without a shadow-stack frame push and without a fn-ref
                 buffer guard: it cannot recurse, cannot reach stack_trace / assert /
                 panic, and cannot sit between the frame that pushed a buffer and
                 the one that releases it.  The live-flip check stays.
```

**In words.** @PLN157 N4.  A runtime fault inside a leaf keeps its exact position
and loses only the innermost frame NAME from the chain.  Switch
`LOFT_NO_LEAF_PRELUDE`.  Site: `Output::is_elidable_leaf` and its use in
`Output::output_function`.

## Deviations

**OPEN: 0** (2026-09-09).

- **D-rw-1 — CLOSED 2026-09-09.**  `one_op_wrapper` asked the body's SHAPE and not
  the definition's ORIGIN, so a user function whose body is one op (`fn reader(w:
  W) -> integer { w.a }`) was emitted as its op; the wasm live-dispatch probe
  flipped it to the interpreter and counted 0 dispatches where it expected 2.  The
  rule was always (R-Wrapper)'s "stdlib"; the code now asks `def.source() ==
  STD_SOURCE`, and cell c11 of `V-o-wrapper-op-cells.loft` pins the user call.
  Found by the GitHub gate on the rebased tree (run 34323456806); the local
  curated set never runs the wasm probe.
