# What to optimise next — the vector-build class, and four priced levers beside it

Taken 2026-09-21 on x86-64, right after the record shapes (`records.md` § Built), from the
portal's 24 rows still over 3×.  An ANALYSIS: every figure below is a HAND-PRICE — the
emitted Rust of the routine edited to the form a rewrite would emit, compiled with the
`--native-release` flags, run in-process with the result hash unchanged — and nothing here
is built.  Two candidates priced NEGATIVE or not at all are listed as such; they are not
work.

| # | lever | rows it moves | hand-priced |
|---|---|---|---|
| **V1** | a reserved counted push loop holds the element BASE and writes the length ONCE | `push`, `comprehension`, `grid`, `f32_build`, every `[for …]` | **−78 %**, **−78 %**, **−76 %** on the three rows priced |
| **V2** | the comprehension's loop is the counted push loop it spells | `comprehension`, `grid`, `drain`'s build, every `[for …]` | −27 % (its share of V1's total) |
| **V3** | the reservation is emitted in the arm that RUNS | `push`, every counted push loop behind a chain guard | −8 % |
| **F1** | a field of a `?`-discharged element — `v[i]?.f` — is one bounds test and one load | `record_update`, `chunk_lookup`'s `map_set`, the idiom everywhere | **−65 %** |
| **T1** | the loop variable of `for p in vector<text>` borrows the element | `join`, `word_count`, every walk of texts | **−47 %** |
| **C1** | a function no fn-ref can reach constructs no fn-ref buffer guard | `fibonacci`, every recursive function | **−27 %** |

## V1–V3: a push loop that cannot grow still pays for growing

`for i in 0..n { v += [i * 3 + salt] }` and `[for i in 0..n { i * i + salt }]` build the same
vector.  Both push through `Stores::push_hoisted`: a capacity test, a store resolved through
`allocations[store_nr]`, the element written, the length bumped in the header AND written
back to the record — per element (`(R-Refresh)`: a runtime reader inside the loop must see
every push).  Against the Rust twin's 0.3–0.75 ns an element, loft pays 2.3–3.1 ns.

Three things are true of these loops that the emission does not use:

- **V3.**  The up-front reservation (`(R-PushFill)`) is emitted only in the CHECKED arm of
  `(R-GuardedChain)`'s guard.  The plain arm — the one that runs — has none and climbs the
  growth ladder.  `Output::chain_fast_path` writes the whole plain loop before
  `push_fast_path` emits the reserve, so the reserve lands in the `else`.  Priced by adding
  it to the plain arm: `push` 48.0 → 45.7 µs.
- **V2.**  A comprehension's loop is named `For comprehension` and has THREE statements —
  the iterator, the element's value, the push — and `hoist::range_counters`, the one parser
  of a counted range, answers only a two-statement loop.  So it is not a counted range to
  any rewrite that reads that parser (no trace names it, because none gets as far as
  declining it): its step stays the null-aware `op_add_int`, its body the fully checked `op_mul_int` /
  `op_add_int`, and it reserves nothing.  Every one of the lane's five comprehensions shows
  it (`make_ints`, `v_comprehend`, `v_grid` ×2, `v_drain`).  The natural spelling is the one
  the compiler should optimise.  Priced by giving it what the explicit loop gets:
  `comprehension` 62.0 → 45.0 µs.
- **V1.**  Once the trip count is reserved, NOTHING in the loop can grow that vector — so it
  is growth-free for the pushed path itself, which is `(R-Base)`'s condition, and the push
  is one store through a held base and a local length.  And where the body holds no reader
  of the vector that goes through the runtime (no call that can reach it), the length need
  reach the record once, after the loop.  Priced with a per-push capacity test kept and the
  header path as the fallback: `push` **48.0 → 10.9 µs (0.73× the Rust twin)**,
  `comprehension` **62.0 → 13.4 µs (2.06×)**, and `f32_build` — whose inner loop already
  holds its reservation and makes eighteen header pushes a pass — **777 → 187 µs (~1.9×)**.

Conditions V1 must validate, and where they already live: the counted range and invariant
end are `hoist::push_loop`'s; "no `break`, `return` or nested loop" is its existing decline;
"no runtime reader of P in the body" is `(R-Alias)`'s exclusivity (an owned local or the
return buffer) plus a body with no call that takes the root; the length written on every
exit is free where the body cannot leave early.  `LOFT_HOIST_VERIFY=1` re-derives the push
header at the end and compares.  The class today: `comprehension` 9.53×, `grid` 8.10×,
`f32_build` 7.50×, `push` 3.03×, `copy` 2.50× — the portal's worst class (7.5×), four of five
rows over 3×.  `copy` is a different mechanism (`vector_add` of a whole vector) and is not
moved by these.

## F1: `v[i]?.field` resolves the store although the loop holds the base

`v[i]?.y` used inline lowers to a JOIN — the element, or the `?` discharge buffer — and then
a general field read of whichever it was: `stores.store(&db).get_float(…)`, in a loop that
holds `v`'s header and element base.  In range the join IS the element, so the field is one
bounds test and one load at `base + i·size + off`; out of range the existing path answers.
Priced on `record_update` (`v[i].x = v[i]?.y + …`, whose WRITE is already fused):
**35.6 → 12.5 µs, 4.63× → ~1.7×**.  It is `map_set`'s shape in `chunk_lookup` —
`m.chunks[i]?.cx == cx && m.chunks[i]?.cy == cy && m.chunks[i]?.cz == 0`, three joins a chunk
— and the spelling a consumer reaches for whenever it wants one field of one element.  The
existing fused element read (`hoist::fused_element_read`) serves a SCALAR vector's `v[i]?`;
this is its record-field twin.

## T1: a walk of texts allocates a `String` per element

`for p in words` binds `p` as `{ … store.get_str(…) }.to_string()` — a heap allocation and a
copy per element — and the walk itself is not hoisted (`vector::get_vector` and
`length_vector` per pass, a checked index step: a text element's head is
`OpGetText(OpGetVectorNullable(…), 0)`, which `hoist::iteration_head` does not read).  The
copy exists because a `&str` borrowed from `stores` would conflict with a later `&mut
stores` in the body.  Where the body writes no store the borrow is sound, which is
`(R-Header)`'s own condition and what `(R-LazySplit)` already does for a parameter nothing
writes.  Priced on the stdlib's `join`: **31.4 → 16.7 µs, 5.36× → ~2.9×**, before hoisting
the walk.

## C1: every call constructs a guard for fn-ref buffers it cannot have

A `--native-release` function enters with `cr_call_push_lean`, a `CallGuard` and
`FnRefBufGuard::new(cell, false)` — the last one two `Cell` reads on every exit, for return
buffers a fn-ref dispatch arm allocated.  `(R-LeafChain)` drops the frame for a call tree
off any cycle and free of fn-refs; a RECURSIVE function keeps all three.  The depth count is
owed (it is the stack-overflow guard); the fn-ref guard is owed only where a fn-ref dispatch
can reach the function.  Priced by removing it from `fib`: **11.8 → 8.67 ms (−27 %),
4.36× → ~3.2×**.  A further −3 % from plain `n - 1` / `n - 2` needs a fact the range proof
does not carry today — a branch condition (`n > 1` in the arm that subtracts) — and is not
proposed on this row's evidence alone.

## Priced negative, or not a clear case

- **`sum` (6.62×) — NEGATIVE.**  Its loop is already a plain counter and a base read; what
  is left is the null-aware checked add, which cannot vectorise.  The admissible route — one
  pass for the element bound (`vector::abs_bound_i64`), then plain adds — measured
  **13.4 → 21.2 µs, slower**: for a single reduction the bound pass IS the cost.  The gap is
  the language's semantics (an overflow's null propagates, C85) against a twin that wraps.
- **`split` (12.27×)** collects `vector<text>`: one owned text per piece against the twin's
  `Vec<&str>`, which allocates nothing.  A representation question (text slices as views),
  one row, and the walked form is already `(R-LazySplit)`'s — a design item, not a lever.
- **keyed (3.5×, ten rows)** is analysed in `keyed.md` § What is left: each remaining item
  is a store-format or data-structure change (the bucket division, a B-tree `index`, a
  chunked `sorted`), priced there at 7–17 % apiece.
- **`mesh_aabb` (4.84×)**: the null-aware float comparison on fields no proof calls non-null
  (`records.md` § 8).  **`par` (5.16×)**: one row, another subsystem; not looked at.

## Order

V1–V3 first: the worst class, four routines priced or sharing the priced loop, every
comprehension in the language, the largest hand-price here, and conditions that three
existing rules already validate.  F1
second — one idiom, −65 %, two lanes.  T1 and C1 are each one clear mechanism worth a
quarter to a half of their rows.  Together they take eight of the 24 rows over 3× to about
or under it.

## Method

`--native-release --native-emit` for the routine's Rust; the edit a rewrite would make,
applied by script to that function alone; `rustc` with loft's own release line (captured
once with a logging `rustc` shim on `PATH`); the lane's own binary run with `--n`, the hash
column compared.  A figure is quoted only where the hash is unchanged and two runs agree.
