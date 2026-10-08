# @PLN125 — First-grade library types

**Status: CLOSED 2026-08-11 — all three arcs shipped.** · **Issue:**
[loft-lang/plans#125](https://github.com/loft-lang/plans/issues/125)

The gaps that stopped a library type being indistinguishable from a built-in one.
**The reference is [INTERFACES.md](../../INTERFACES.md)** — the measured
capability table there now has none left, and each arc has its own section beside
it. This directory is the closure record: what shipped, what the instruments
found, and the two pieces that turned out to belong elsewhere.

| arc | what | shipped as |
|---|---|---|
| **A** | associated types — an interface names a companion type | `type Rows: Cursor` + `Self.Rows` |
| **B** | a hook at scope end | `fn OpDrop(self: T)` |
| **C** | `x[i]` dispatched to a library type | `fn OpIndex(self: T, i: τ)`, `op []` |

Catalogue: [`@F113`](https://github.com/loft-lang/features/issues/113) ·
[`@F114`](https://github.com/loft-lang/features/issues/114) ·
[`@F115`](https://github.com/loft-lang/features/issues/115).
Behaviour matrices: `tests/scripts/pln125-{a2c-companion,b-drop,c-index}.loft`.

**Two pieces left, and neither is this plan's:**

- **A4** — collapsing @PLN124's `hole_*` family needs a GENERIC METHOD, not
  associated types. Filed as @PLN137 and then **DECLINED** (@C110): the collapse
  would delete a deliberate safety refusal, and the first-parameter rule it needed
  lifted is the monomorph's identity. The reasoning for the premise being wrong is
  below, because the plan's own claim was that these were one feature.
- **A3** — the SQL cursor split is a library migration, now unblocked. Filed as
  **@PLN138** and since **SHIPPED**: `SqlRows` + `type Rows: SqlRows`, all four
  backends, `schema.loft`'s index walk nested rather than buffered, and two
  shims changed from one-per-process to one-per-cursor. The language claim it
  existed to prove ("two cursors coexist") is proved in
  `pln125-a2c-companion.loft`; the same claim on a real driver is
  `tests/fixtures/sqldb/two_cursors.loft`.

  It is also what measured arc A and arc B against a consumer neither had, and
  three defects came back: a private `OpDrop` in a library never ran anywhere
  (fixed here — `Data::drop_hook_nr`), an associated-type monomorph freed a stack
  record when the implementor's producer delegated its return (loft#847, fixed —
  the companion binds without deps), and a droppable copied into a struct or enum
  field is released at the source and never through the container (loft#849,
  open, worked around with `disown`).

Each arc landed **inert first**: the contract declared, every existing program
proved byte-identical in IR and native Rust, before any behaviour routed through
it. `bytecode-comparisons/` holds those instruments.

---

## Arc A — the record

`type X: B` parsed (A1), an implementor's return type became part of satisfaction
(A2a), and `Self.X` in a signature resolved to a placeholder (A2b). What was
missing was the step that makes the placeholder mean anything — before A2c:

```loft
fn first_width<S: Source>(s: S) -> integer {
  r = s.open();          // r : Source.Rows — a placeholder
  return r.width();      // error: generic type Source.Rows:
}                        //        field access requires a concrete type
```

**A2c's invariant:**

> An associated type is a type variable owned by the interface. Inside a generic
> it dispatches through its declared bounds exactly as `<T: I>` does, and at
> instantiation it binds to the one concrete type the implementor's methods
> agree on — which must satisfy those bounds.

Phrased that way A2c is not new machinery: it is the bounded-type-variable
machinery applied to a second holder. A monomorph already substitutes one
`(type variable → concrete)` pair through the template's code, variables,
attributes and return type; A2c makes that a LIST and appends one pair per
associated type.

**A2c is SHIPPED.** Three parts, each the second user of something that already
existed:

| part | where | reuses |
|---|---|---|
| the template can CALL the companion's methods | `create_bound_method_stubs` now builds stubs for an associated type too | the `t_<LEN><Holder>_<method>` stubs a bounded `<T: I>` already gets |
| the monomorph BINDS it | `associated_bindings` + a list of `(holder → concrete)` pairs in `instantiate_generic` | `substitute_type` / `substitute_type_in_value` / `Function::substitute_type`, applied once more |
| the declared bound is CHECKED | `check_satisfaction` asks the same structural question of the companion | `satisfaction_failures`, extracted so both askers word their own message |

The companion is inferred from the implementor's own signature, read back through
the interface's — where the interface writes `Self.Rows`, the implementor writes a
concrete type, and that is the companion. Nothing else can say it: satisfaction is
structural, so there is no `impl` block to name it in. Return AND parameter
positions are read and must agree; disagreement is refused rather than resolved by
declaration order.

**The gate it was built against:** the monomorph of `bound_width<S: Source>` is
byte-identical to `hand_width(s: FileSource)` written out by hand — same locals,
same retbuf, same frees, same order. Only the live-dispatch preamble differs, and
no monomorph has ever carried one.

Two refusals ship with it, both in `tests/parse_errors.rs`:

```
'S1' binds 'Source.Rows' to 'Bad', which does not satisfy the declared bound 'Cursor': missing width
'S2' does not agree with itself about 'Source.Rows': 'open' and 'feed' name different types for it
```

The first names all four parties on purpose — three of them are invisible at the
call site, where the reader only wrote `use_it(s)`.

An associated type's name is now enforced CamelCase like every other type name.
It stopped being cosmetic when it became the `t_<LEN><Interface>.<Name>_<method>`
stub name whose LEN prefix is parsed back to recover the method: an underscore in
it would split that name in the wrong place and leave the call on the template
stub, which native emits as `todo!()`.

## What the instrument found first

Building the before/after corpus for A2c surfaced three defects in the code path
arc A depends on — all of them already on `main`, none of them filed, because
the whole point of arc A is a method that RETURNS a companion struct and that is
the path they sit in. Written down before any of them was worked:

| # | shape | symptom | on A2c's path? |
|---|---|---|---|
| **1** | an interface method returning a struct/vector/enum **declared later in the file** | `Incorrect var __ref_2[65535]` — an internal compiler error, BOTH backends | **yes** — `Self.X` is a forward reference by construction |
| **2** | a bounded-generic **operator** returning `Self`, consumed inline (`assert(op_of(i,j).w == 10)`) | one store per call not freed | no — a method-shaped return is clean |
| **3** | an interface's associated-type placeholder | minted as a METHOD stub (`t_1D_SqlDb.Rows`), which native emits as `todo!()` | no — dead weight, not wrong output |

**#1 is the blocker.** A bound-method stub stands in for the concrete method
`re_resolve_call` will substitute, so its ABI must equal that method's ABI — and
the hidden `__retbuf` parameter that a struct/vector/enum return carries was
decided on the FIRST pass, from a return type that was still an unresolved
forward reference, and never revisited (the second pass sees the stub already
exists and skips it). The monomorph then gained a `__ref_N` with no scope and no
initialiser, so it never got a stack slot.

The fix re-derives every stub's signature BETWEEN the passes, where all types
resolve (`refresh_bound_method_stubs`). The first cut refreshed it in pass 2 and
the **H5 two-pass guard rejected it** — growing an attribute there is exactly the
cross-pass divergence H5 exists to prevent — which is how it ended up in the same
place #675 already put the same question about a function's own return. Wherever
the two passes already agreed the re-derivation is idempotent, which is what
`bytecode-comparisons/bound-stubs-corpus.loft` proves.

**#3 is a one-line sibling.** `check_satisfaction` already tells an interface's
method stubs from its associated-type placeholders by `DefType` — the stub
builder walks the same children and did not.

**#2 is left open** — it is a leak, not a corruption, it is narrower than it
first looked (a method-shaped return, a bound local, and the non-generic
operator are all clean), and it is not what arc A needs. Recorded here rather
than fixed inside another change.

A fourth surfaced later, while probing A4, and is **fixed**
([loft#845](https://github.com/loft-lang/loft/issues/845)): a format string picks
its op from the value's TYPE, and a template has only the parameter's, so a
monomorph ran the template's choice against a concrete value. A bare `T` took the
RECORD formatter — SIGSEGV on `--interpret`, `E0308` on `--native`, and the
literal `{}` for `text` and structs — and a `vector<T>` took the right op with the
type variable's ROW. The two got different answers: a bare `T` is refused
(`<T: Printable>` is the cure and renders every kind), and a `vector<T>` is fixed
by re-deriving the row per monomorph from the VALUE's own substituted type.
Reproduced on the installed `2026.8.0`, so it predated this work.

---

## Arc C — shipped

The reference lives in [INTERFACES.md § Indexing](../../INTERFACES.md); what
belongs here is why it was three small pieces rather than one.

`x[i]` had exactly one answer for a struct — the keyed-collection refusal — and
`index_type` (the TYPE of `x[i]`) and `parse_index` (the CODE) are two functions
that had to start agreeing. So the lookup is one home, `user_index_op`, and both
read it: a type that answers one must answer the other or they disagree about
what indexing means.

Three refusals came with it, and each is a case that would otherwise have been
silent or misleading:

| shape | why it is not a fall-through |
|---|---|
| a struct with no `OpIndex` | the old message sent the reader to a `hash<Row[id]>` constructor unrelated to their struct; the cause is one missing signature and the message now names it |
| an UNBOUNDED `<I>` subscripting | a bound stub is named for the HOLDER (`t_1I_OpIndex`), and holder names are shared — a sibling `fn a<I: Indexable>` mints exactly the name `fn b<I>` would find. Measured: `b` compiled and worked, promising nothing. The binary-operator path carries the same guard for the same reason |
| `x[i] = …` | the assignment path reported *"Cannot assign to attribute on type `t_4Bits_OpIndex`"*, naming an internal symbol the author never wrote |

**A writing counterpart is deliberately not here.** It needs its own method and a
decision about whether `x[i] += 1` may then read-modify-write. Refused with a
message that names the alternative, rather than left to a confusing error.

The `op []` spelling is handled where an `op` has just been read and a `[` can be
nothing else: the lexer has no `[]` token, and it must not have one, because `[`
and `]` are separate everywhere else.

## A3 and A4 — the premise was wrong, and that is the finding

The plan says:

> A3 and A4 are the proof that this is one feature and not two — a companion type
> and a generic method parameter are the same gap seen from two sides, and if
> only one of them lands cleanly the invariant is wrong.

**They are not the same gap.** Measured after building A2c, with the compiler's
own words as the evidence:

```loft
struct Acc { n: integer }
fn feed<T>(self: Acc, v: T) { … }
```
```
Type variable T must appear in the first parameter —
move T to the first parameter position
```

A4's `fn hole<T>(self: Self, v: T)` is refused by a deliberate, explicit rule, and
lifting it is a change to how generics DISPATCH: `re_resolve_call`,
`instantiate_generic` and the H5 bound-stub recogniser all key on
`attributes()[0]`, and the monomorph's mangled name is built from the first
argument's type. That is **generic methods**, a separate language feature that
A2c neither needed nor provided — A2c reuses the existing single-holder
substitution and adds holders to the list; nothing about it makes a type variable
addressable in a later parameter.

So the plan's stated test of the invariant cannot be run as written. The honest
replacement: **arc A's invariant is proved by A3's claim, and A3's claim is
proved** — `tests/scripts/pln125-a2c-companion.loft::two_cursors` shows two
companions alive at once from one connection, which is exactly the limitation
that put @PLN23's cursor ON the connection. What remains of A3 is the LIBRARY
MIGRATION, not the language proof.

**A3's remaining work, sized:** the cursor API (`db_select` / `db_next` /
`db_col` / `db_width` / `db_rows`) has ~136 call sites across 14 files — the
`sql` interface, five backends, the `schema` layer and the fixtures. Three of the
four `#c` backends run on a developer box (sqlite / maria / postgres); **duckdb
cannot be validated here** (`libduckdb.so not installed`), so a migration would
change a working, tested API on a backend nobody can run. That is the reason it
is left open rather than the size.

**A4 is blocked** on generic methods and should be tracked as such rather than as
part of this plan — the interpolation `hole_*` family stays per-kind until a type
variable can be addressed outside the first parameter.

---

## Arc B — shipped

The reference lives in [INTERFACES.md § Running at scope end](../../INTERFACES.md);
what belongs here is the record of what the design predicted, what it got right,
and the two places it needed sharpening.

**The prediction held.** "A drop runs exactly where the value's own `OpFree*`
runs" turned into one call pushed into `get_free_vars`, and eleven of the
thirteen shapes were then correct with no further code — early `return`, `break`,
return-out-of-a-loop, reverse-declaration order, a returned value not firing in
the callee, a borrowed argument not firing at all, one per loop iteration, value
structs, and both backends producing identical traces. That is what "derives from
the borrow model rather than sitting beside it" buys.

**Two sharpenings, both found by running the matrix rather than by reading:**

| what the design said | what was missing |
|---|---|
| the drop goes where the free goes | **the free is null-tolerant and a drop is not.** `OpFreeRef` on a slot never written is a no-op — it checks `rec == 0` and returns — so the emitter never had to know whether a binding held anything. A drop is a USER call, and `if n > 0 { t = Tx { … } }` ran the author's rollback on the else path against a record that does not exist. The call is now wrapped in the same liveness test the free performs internally. |
| the drop goes where the free goes | **the free is about the STORE, the drop is about the BINDING.** A value delivered through a caller-side return buffer has two variables naming one record: the buffer (`__ref_N`, function-scoped and REUSED across iterations) and the witness the author bound. `t = begin(…)` in a loop freed through `OpFreeRefIfDistinct(t, buffer)` — a branch the first cut skipped — so a loop opening a transaction every pass rolled back once, at the end. The witness drops and the buffer does not. |

The second one is the dangerous shape and the reason B5 asked for a matrix
rather than a demo: it is silent, ordinary code, and wrong in the direction that
loses work.

**One spelling changed from the sketch.** The design proposed `#drop fn`; it
ships as `fn OpDrop(self: T)`. Attributes in loft describe a function's
IMPLEMENTATION (`#pure`, `#native`, `#c`), and every other first-grade surface
keys behaviour to a TYPE by the method's name — `to_text`, `OpAdd`, `next`,
`lit`/`hole_*`, and now `OpIndex`. A scope-end hook is one more of those. It also
needs no `Definition` field and so no IR-store schema change.

**What the matrix documented that no one had written down:** a struct field
COPIES at construction, and a drop receives only `self` — so a drop cannot write
back into a caller's loft-side collection at all. Its effect reaches the world or
nowhere. That is exactly right for the motivating case (a `#c` `ROLLBACK` on a
handle the transaction owns) and a real limit worth stating, which is why the
test's trace is a file rather than a vector.

**Consumers (B5): two, unrelated.** A transaction, where `commit` ANSWERS and the
closing brace does not — the asymmetry the whole design rests on — and a lease,
which has no explicit release at all. Nothing about the second resembles the
first, which is the point: one consumer leaves the invariant tested by one shape.

---

## Instruments

| file | what it is for |
|---|---|
| `bytecode-comparisons/bound-stubs-corpus.loft` | one generic per stub-ABI branch (scalar / text / struct / vector / operator / shared method name). The before/after diff must be EMPTY for any change that only reshapes the stub builder. |
| `bytecode-comparisons/a2c-companion.loft` | the hand-monomorphised reference — the IR A2c's monomorph must equal. |
| `bytecode-comparisons/a2c-companion-bound.loft` | the same body written against the interface. The diff between the two is A2c's specification. |
