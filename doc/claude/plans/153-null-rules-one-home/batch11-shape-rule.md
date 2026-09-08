# Batch 11 — the `matches!` generalization: one rule, four verbs, one ratchet

## The invariant (already written, nowhere enforced)

`Type::peel_optional`'s own doc block states it:

> "the nullability-agnostic MAJORITY of `match Type` sites peel through this; only the
>  discharge / store / cast checks (N-Store/N-Decl/N-Coal/N-Match) read the bool."

Restated as the rule this batch adds:

  (N-Shape)  a question about a type's SHAPE — what storage it uses, which definition it
             names, whether it reaches a store, what it borrows — answers IDENTICALLY for
             `τ` and for `τ?`.  Only a NULLABILITY question may differ, and it is spelled
             by reading the bool, never by the absence of an arm.

It follows from C90: `Optional(τ)` shares `τ`'s storage; the `?` is a compile-time marker.

## Why the design's own guarantee does not fire

`Optional` was made a `Type` VARIANT on purpose (data.rs) — *"so every exhaustive
`match Type` is a COMPILE ERROR until it handles nullability (loud omission)"*.
`matches!`, `if let` and a `match` with a catch-all are the escape hatch, and the audit
counts **2267** shape tests, **1523** opaque on their own scrutinee.  A 1523-site hand walk
is not a plan; the guarantee has to come back at the HOMES, not at the sites.

## The three moves

- **A — the four blind SHAPE verbs peel.**  `is_dbref` (23 bare callers), `is_scalar` (6),
  `heap_dep` (10), `heap_def_nr` (9 bare / 23 hand-peeling).  Each is a shape question by
  its own doc, and each already has a peeled TWIN or hand-peeling callers — `is_dbref`'s doc
  names its own twin (*"`Parser::is_heap_handle` is the same question with a `.base()` peel"*).
  Fixing the verb closes ~48 bare call sites at one home.
- **B — the rule.**  `(N-Shape)` in formal/types.md, cited at the peeling accessors and the
  four verbs, so *"may this site ask bare?"* becomes a lookup.
- **C — the ratchet.**  `ir_walker_audit.py optional` gets a frozen baseline and a gate, so
  the opaque count cannot GROW.  It cannot retire 1523 sites; it can stop paying for new ones.

## Deliberately EXCLUDED, and why (the "equal today is not the same rule" note)

- `is_unknown` — 106 bare callers and they are RIGHT: phase 0 F1 settled that a wrapper over
  a stub IS a written type at the settledness guards; `has_unknown` is the walking twin.
- `find_fn` — `(F-Recv)` keys two overloads apart BY the `?`; peeling would merge them.
- `fmt` / `show` — must render the `?`.
- `borrow_deps`, `rewrap_deps` — AUDIT FALSE POSITIVES: their own match names `Rewritten`
  with no `Optional` arm, but they delegate to `deps_ref` / `with_deps`, which are
  dep-transparent by construction (`@PLN25 — Optional is dep-transparent`).  Recorded so the
  ratchet's baseline is honest rather than merely frozen.

## Prediction, written BEFORE the probe (this is what the corpus run may falsify)

Peeled one verb at a time, `scripts/introspect_diff.sh` over the 1411-file corpus:

| verb | predicted | reasoning |
|---|---|---|
| `is_scalar` | 0–5 files | loft#1372 already peeled the nine `&`-lowering sites; the 6 left are in the native emitter |
| `heap_def_nr` | 5–20 files | it is `(B-Copy)`'s home — a bare "no" makes a nullable bind ALIAS where the dense twin copies (loft#1319's shape, one site of it) |
| `is_dbref` | 10–30 files | the most bare callers, and it routes a handle down the SCALAR path when it says no |
| `heap_dep` | 5–20 files | scope-exit free placement; a change here moves FREES, the highest-risk channel |

**Falsified if** the total is large enough that the differences cannot be read one by one
(> ~40 files), or if any verb's peel breaks the corpus rather than moving it.  In that case
the verb is staged on its own switch instead, and A is cut to the verbs that measure small.

**Also falsified if** a verb measures ZERO everywhere — that would say the bare answer is
never asked with a `τ?` in production, which makes the peel insurance rather than a fix and
moves it behind C (the ratchet) in the ranking.

---

## Measured (2026-09-08) — the prediction table, scored

| verb | predicted | measured | verdict |
|---|---|---|---|
| `is_scalar` | 0–5 files | **1** — batch 3's own "unreached" cell; the hoisted declaration moves into the native prologue | ✅ landed |
| `heap_def_nr` | 5–20 | **8**, every one a null guard, all identical in value on both backends | ✅ landed |
| `is_dbref` | 10–30 | **102**, and **12 guards break** — ≥13 callers read the blindness as a nullability test | ❌ falsified → its own walk |
| `heap_dep` | 5–20 | **0** (IDENTICAL 1411/1411) | ✅ landed as insurance |

Two of four wrong, and both wrong ones changed the plan — the `is_dbref` peel was cut by the
threshold this document wrote down before the run, and `heap_dep` landed as insurance rather than
as a fix, exactly as the "also falsified if a verb measures ZERO" clause said it should.

Combined, the change reads **DIFFERENT 9 of 1411**, and the two site fixes it carries
(`channel_0_carries`, `nstore_null_report_as`) add nothing to that number — byte-identical, which
is the proof that spelling a nullability test explicitly changes no outcome today.

### The `is_dbref` walk, for whoever takes it

The cure is not a peel; it is a SPLIT.  Its callers ask two questions through one predicate:

- *"what shape is this?"* — peels; write `is_dbref(tp.base())` or `Parser::is_heap_handle`.
- *"is this a non-null heap slot?"* — must spell the `?` test itself, in BOTH spellings
  (`Type::Optional` and the synthetic `__nullable<S>`), the way `nstore_null_report_as` now does.

Start from the 12 guards the blanket peel breaks — they name the callers, and each is a
store-lifetime or null-gate decision, so each wants a cell rather than a reading.

## Recorded on the way: loft#1450's remaining leg, RE-measured

`types-history.md` says the leg costs "37 corpus sites".  That estimate predates `D-col-lookup`,
which made every keyed lookup `τ?`, so every `h[k].f` now arrives nullable too.  Re-measured on
this tree with the projection typed through `(N-Prop)`: **115 unique sites across ~40 corpus files
plus `lib/code.loft` and `lib/lexer.loft`**, of which 14 files stop COMPILING (a nullable
collection cannot be iterated, `len`-ed or `.remove`-d; a narrow-width field read through a
nullable receiver is a hard `(N-Store)` error).

It also surfaces two sub-defects that must close FIRST, because both stop the program parsing
rather than warn:

- a fn-ref FIELD called through a nullable receiver (`es["a"].f(3)`) — a syntax cascade;
- an `is` PATTERN on a nullable enum field (`if r.r_shape is Box { bw, bh }`) — the same.

And one that is not the `?`'s fault but is measured beside it: flow narrowing does not exist on
PASS 1 (a `s != null` condition lowers to a bare `Var` there), so a guarded read inside a
value-`if` widens the INFERRED local to `τ?` and the type never narrows back.

So the leg is a 2–3 day arc, not a batch.  The estimate in `types-history.md` should be read as
stale rather than as a cost.
