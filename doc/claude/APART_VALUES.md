# Apart values — fast representations that never touch the store model

**A proposal: no code follows it until the owner accepts the rule below.**

The store model is an in-memory database built for data that LASTS: records with identity, a
best-fit allocator with a free tree, ownership walks so nothing leaks, checked access because a
record may be shared or persisted.  Every loft data structure lives in it, and that is right.  A
value that lives one iteration pays the whole protocol anyway, and those throwaway values are
what keeps loft far from rustc.  This note proposes representations of the compiler's choosing
for such values, kept entirely APART: invisible to loft's semantics and to the store model,
meeting the store only through its ordinary constructors at a boundary.

## The problem, measured

One throwaway per iteration, `--native-release` against `rustc -O`, the sums equal (arm64 macOS):

| throwaway shape | loft | Rust |
|---|--:|--:|
| record of scalars (`(R-ValueRecord)`'s tuple applies) | 0 ns | 0–1 ns |
| record with a 4-element vector field | 84 ns | 20 ns |
| a 4-element vector, built and summed | 27 ns | 1 ns |
| an enum with a text variant | 52 ns | 7.5 ns |

The moment a throwaway owns heap it pays, per iteration: a release walk over its type, a delete
into the free tree, a best-fit claim of the same size straight back, a default prefill, and a
store lookup per field access.  The profile of the second row is that list and little else.
The same list is 25–40 % of cbor `decode` (bench/portal/analysis/under-10x.md § cbor `decode`).

**The prize.**  The third row's function, hand-written in its emitted Rust as a stack array
`[i64; 4]` with the same overflow-checked additions: **29 ns → under 1 ns**, the sum unchanged
(`hand_price.sh`, three alternated rounds).  The whole cost was the store traffic.

## The rule

```
  (R-Apart)      a value the compiler proves is never OBSERVED AS A STORE VALUE may be
                 held, on --native, in any representation of the compiler's choosing —
                 on the stack, on Rust's heap, in registers — that has no
                 meaning in loft's semantics and no entry in the store model: no
                 DbRef, no identity, no release a program can see.  It is LIVE from
                 its construction to its first BOUNDARY, where it is converted through
                 the ordinary store constructor into the value the unrewritten program
                 would hold, and from then on only the store form exists.  Every use
                 between the two is on an ALLOW-LIST whose answers equal the store
                 path's, faults included (H-Index, H-WriteOOB); a value with any other
                 use is not admitted and lives in the store from its construction.
```

**Why loft may do this.**  A loft program specifies WHAT the programmer wants, not how each
step is carried out; the specification is the program read through the formal rules, and the
compiler may realise it any way that answers the same.  An optimising C or Rust compiler works
under the same "as if" freedom, but it has to reconstruct facts loft states outright: the type
of every value and its layout, which stores can never alias (`(R-Alias)`), every owner and every
lifetime (the scope pass), and — in a release build — the whole program as one unit (C122).
What LLVM must prove by escape analysis, loft already knows, so it can choose representations
LLVM never dares to.  That freedom depends on loft having no construct that observes
representation (C135): the two places it is observable, a package's native code and a store
bound to a file, are exactly where an apart value must already be a store value.

**Where it sits.**  It is an instance of `(R-Escape)` (C122): the contract is what a program
computes and observes, never where a value lives.  It does not revisit C125: `(O-One-Kind)`
governs every value that reaches loft's heap as a `DbRef` naming a store, and an apart value
never gets one.  `(R-ValueRecord)`'s tuple in registers is the existing instance; this note
extends the same idea to values that own a collection.

## What "apart" requires

1. **One live representation at a time.**  The conversion at a boundary is one-way.  An apart
   value is never converted and then used again in its apart form, so two copies cannot
   diverge.
2. **A distinct Rust type.**  An apart value is never encoded in a `DbRef`-shaped form (a fake
   store number, a sentinel record).  Any store routine handed one is then a `rustc` type error,
   not a wrong answer.
3. **One admission gate.**  A single function over the IR decides admission from the value's
   whole life: its construction, every use, every assignment.  33 operators take a vector
   operand, plus the stdlib functions that do; teaching each an apart form would be 33+
   re-assertion sites of this rule.  The ALLOW-LIST collapses them: a use not on it declines
   the value, so an operator nobody taught costs the rewrite, never correctness.  The distinct
   Rust type (2) is the backstop if the gate itself is wrong.
4. **The same answers.**  Each allowed use answers exactly what the store path answers — a
   negative index counts from the end, an out-of-range read answers null (`@FR-H-Index`), an
   out-of-range write does nothing (`@FR-H-WriteOOB`), checked arithmetic stays checked.
5. **Native only, decided once.**  The interpreter has no Rust values to choose from and keeps
   the store.  The proof lives in the shared IR, so both backends agree on where the rule
   applies; only native acts on it.
6. **A switch and a falsifier.**  `LOFT_NO_APART=1` puts every value back in the store — the
   before-half of every A/B and the first bisect step for a native-only wrong answer.

## The boundaries

A value crosses into the store when it is:

- stored into a record field or a collection element;
- passed as an argument where the callee takes a store value (until a callee is admitted to
  take the apart form — instance 2 below);
- returned where the caller expects a store value;
- tested for identity or null (an apart value is never null: a `vector?` is not admitted);
- persisted, sent to a native (`#rust`) function, or captured by a closure;
- live across a `yield` or handed to a `par` worker (not admitted at all in a first build).

## Failure paths, and what holds each

| failure | guard |
|---|---|
| an operator not taught the apart form receives one | the gate's allow-list declines the value; a gate bug is a `rustc` error (requirement 2) |
| the value is converted at a boundary and the apart copy is used again | the conversion is one-way (requirement 1); a cell that reads after a boundary |
| a read or write answers differently out of range or with a negative index | cells per allowed use with hand-computed values, both switch states |
| the length grows past what the apart form holds | instance 1 admits only a length proven constant for the value's whole life |
| an element owns heap (a `vector<text>`) | not admitted in instances 1 and 2 |
| the gain is eaten by the conversion | measure the boundary, not only the throwaway (next section) |

## Where it pays — the bench routines

The design question is less how fast a temporary can be (Rust already answers that) than
**where the boundary falls and what crossing it costs**.  A stack vector that saves 25 ns and
pays 30 ns to be converted has gained nothing.  The measured routines fall into three shapes:

| routine | throwaway | boundary | the fix |
|---|---|---|---|
| vector-temp (`sum4`-shaped helpers, float kernels) | a local vector, summed or read | none — consumed where made | **apart (instance 1)**: priced 29 → <1 ns |
| mesh3d `mat4_mul` (4.68×) | `Mat4 { m: [16 floats] }`, rebound every iteration | once, when the last product is returned | **apart (instance 2)**: the vector field inside a value record, across admitted calls; to be priced |
| cbor `decode`, 17_consumer `emit_to_material` | each child / triangle | at once — built to be stored in a parent | **not apart**: the value is made to be stored; destination passing (build it in its element, D7) |

So the rule covers values consumed where they are made.  Values made to be stored are
destination passing's, and a buffer reused per call site is the store's own business (reset
instead of release-and-reclaim, a store-internal change with no new representation).  The
three together cover the alloc-temp, vector-build, record-build and record-field classes,
the four worst in the portal.

## Instances, in the order they would be built

1. **Fixed-length scalar local vectors that never cross** — a `[T; N]` stack array for a
   `vector<T>` local of a scalar `T` whose length is the same constant for its whole life: a
   literal of `N` elements, then only index reads and writes, `len`, and `for` iteration.
2. **The same vector as a field of a value record**, across calls the gate admits on both
   sides — `(R-ValueRecord)`'s tuple carrying the array.  `Mat4` is the case.
3. **Bounded variable-length vectors and texts** (a small inline buffer, or a Rust `Vec` /
   `String`) — only if instances 1 and 2 leave a measured class above the bar.

## Verification

- Cells per allowed use × both switch states, hand-computed, values AND lengths; a cell per
  boundary that reads the value after it crosses.
- A planted defect per requirement (a use admitted that should decline, a conversion that is
  skipped), each failing a cell.
- `make perf-portal` before and after: the class medians of vector-build, float-kernel and
  alloc-temp should move, and no long-lived class may get slower.
- `make rewrite-census`: no existing rewrite fires less often.

## Which representations are allowed

Any representation loft itself cannot express — on the stack, on Rust's heap, in registers — as
long as the boundaries above hold: it is never observable by a program (C135), it meets the
store only through the ordinary constructors at a boundary, and every allowed use answers what
the store path answers.  A stack array is one instance, not the rule; a Rust `Vec` or `String`
allocated outside the stores is equally admissible (owner).  The choice per value is the
compiler's, made on measurement.

The same holds for what a use DOES: any implementation is allowed as long as the user cannot see
the difference.  Formatting an apart value (`"{v}"`, `log_info`, `println`) is an allowed use,
not a boundary — it may format the apart form directly, provided the text is byte-identical to
what the store path prints (brackets, separators, number formatting, null rendering).  That
equality is the guard: cells that format each apart shape both ways and compare the bytes.

## Open questions for the owner

1. One new rule `(R-Apart)`, or an extension of `(R-ValueRecord)`?  This draft prefers a new
   rule that `(R-ValueRecord)` becomes an instance of, since the tuple is the first apart form.
