# OCAML_BAR — expressiveness probes measured against OCaml

**Status of this document:** a bar, not a plan. It lists things OCaml expresses
directly that loft, as documented on 2026-09-21, does not (or does not
verifiably). Every entry is a small program that either runs green on both
backends or is refused with a named diagnostic. Nothing here is a commitment to
implement; it is a ruler the project can be held against, and re-measured after
every change that touches generics, closures, patterns, dispatch or nullability.

**Audience:** the coding agent. Read this whole file before touching any probe.

**Method.** The surface was inspected from the public reference
(`loft-lang.org/loft/`: `25-generics`, `26-closures`, `29-match`, `09-enum`,
`33-features`, `stdlib-interfaces`). Statuses below are what those pages state
or fail to state. The first job of the agent is to *re-measure* — run every
probe, replace each `Status:` line with the real result, and commit that as the
baseline. Where the docs and the compiler disagree, the compiler is the truth
and the doc page is a bug to file.

**That measurement has been taken** (§ Evaluation, below): every entry now carries a
`Measured 2026-09-21:` line above the documentation-derived one, and where the
measurement contradicts a rule in this file, the Evaluation's corrections win.

---

## Evaluation — measured 2026-09-21

Every probe was run on both backends against `9f5cf6a96` (origin/main) plus the loft#1572 fix,
from a scratch directory; no probe file or harness is committed yet. **The two backends agreed on
every cell**, so each row below is one answer. Where the probe's spelling was wrong for a
capability loft already has, the row scores the capability in its real spelling and says so. The
document's own rule — *a probe that cannot parse is a FAIL* — would otherwise have recorded four
false FAILs (A7, B5, C8, F2).

| entry | measured | what answered |
|---|---|---|
| A1 map, two variables | FAIL | `<T, U>` does not parse — @PLN165 arc C |
| A2 fold | FAIL | same — arc C |
| A3 variable not in first param | FAIL | same — arc C |
| A4 generic struct | FAIL | `struct Pair<` does not parse — @PLN165 arc D |
| A5 generic recursive enum | FAIL | arc D, and see A5b |
| A5b recursive enum, monomorphic | FAIL | refused: *"Enum 'Expr' contains itself … use reference<Expr> to break the cycle"* — and a `reference<Expr>` field then refuses `&e` (**defect 3**, loft#1579). A `struct Box { e: Expr }` pointed at through `reference<Box>` builds and evaluates a tree on both backends |
| A6 user `Result` | FAIL | arcs C + D |
| A7 user-declared interface | **PASS** | the spelling is `fn area(self: Self) -> float` (INTERFACES.md); the probe's `fn area(self)` was the defect |
| A8 associated type | PARTIAL | the companion works inside a generic (`type Rows: Cursor`, `Self.Rows`, @PLN125 — PASS); naming it in the generic's OWN signature (`-> S.Rows`) does not parse, as INTERFACES.md documents |
| B1 closures in a vector | FAIL | named refusal: *"a capturing closure cannot be stored in a collection: a collection has one element layout and each capture set is its own record shape"* |
| B2 handler table | FAIL | B1, and `hash<(…)>` has no tuple element type |
| B3 two closures, one written scalar | FAIL | named refusal: *"mutated through a closure and captured by 2 closures"* — design-negotiable as the entry says |
| B4 local recursive function | FAIL | *"'fn' definitions must be at file scope"*; the self-referencing-lambda spelling is an **internal compiler error** (**defect 2**) |
| B5 capture a `&` parameter | **PASS as `pass`** | capturing and writing through a `&` parameter WORKS on both backends (`[1,10]` → `[2,11]`); the refusal the probe expects does not exist and 26-closures is stale (**doc bug 5**). The probe as written was also invalid (`bump_all(&xs)`, an untyped `\|i\|`) |
| B6 compose | FAIL | arc C |
| C1 nested constructor pattern | **PASS** | `W { i: A { v: 0 }, k } => …` over a NON-recursive nesting, both backends; the recursive shape is blocked on A5b only |
| C2 literal in field position | **PASS** | `Circle { r: 0.0 } => "point"` works (the reference page does not show it) |
| C3 field rename + as-binding | FAIL | `Rect { w: width, h }` → *"Unknown variable 'width'"*; `whole @ Rect {…}` → *"'whole' is not a variant"* |
| C4 tuple of variants | **PASS** | `(Playing { hp }, Damage { amount }) if hp <= amount => …` works on both backends; the probe failed only on its arm BODY `Playing { hp }` — field-init shorthand, which loft does not have (`Playing { hp: hp }`) |
| C5 or-pattern with bindings | FAIL | `Circle { r } \| Sphere { r }` does not parse |
| C6 exhaustiveness through nesting | PARTIAL | over a non-recursive nesting the hole IS refused, but named by its outer variant (*"not exhaustive — missing: W"*), not as `W { i: B }` |
| C7 scalar-match hole diagnosed | FAIL | silent, both backends |
| C8 head/tail pattern | PASS with `_` | slice patterns exist (`[]`, `[x, ..rest]`); `[]` + `[x, ..rest]` is not seen as total (*"a slice pattern can fail … add a '_ =>'"*). The probe's own `fn sum` collides with the reserved stdlib name |
| D1 generic return smuggles null | FAIL — a DECIDED edge | `formal/types.md` (N-Index) trusts a constant index *by contract* (C80, @PLN102 D1): `v[0]` reads `T` non-null and faults to null at run time. Moving this bar reopens that decision; it is not an unregistered hole |
| D2 empty-stub default | FAIL | silent `0.0`, both backends |
| E1, E2 structural sharing | BLOCKED | on A5b (no recursive enum can be built); `memory_used()` does not exist — `store_memory()`'s record count is the instrument the corpus uses |
| F1 missing combination named | PARTIAL | refused at compile time: *"no definition of `beats` takes (Hand, Hand) — declared: beats(Rock, Scissors), …"* — names the declared set, not the missing pair |
| F2 enum-level fallback | FAIL — **defect 1** | on a PLAIN enum every one of the nine combinations answers the fallback, silently; the same program over struct-enum variants dispatches correctly (`R>S`, `P>R`, `S>P`) |
| G1–G13 regression floor | covered | every cited page IS a test: `tests/docs/*.loft` generate the reference pages and run in `make ci`, so the floor is already gated |

**Score (capability, both backends):** A 1/8 + 1 partial · B 1/6 · C 4/8 + 1 partial · D 0/2 ·
E blocked · F 0/2 + 1 partial.

### Defects the measurement surfaced (filed)

1. **loft#1577 — multiple dispatch ignores per-value definitions on a plain enum** — `silent-wrong`. `fn beats(a: Rock, b: Scissors)` on `enum Hand { Rock, Paper, Scissors }` is accepted and never chosen: with an enum-level `beats(a: Hand, b: Hand)` all nine combinations take it; without one, the call is refused although definitions match the runtime values. Struct-enum variants are unaffected. Either the declaration is refused or the dispatcher reads the value.
2. **loft#1578 — a self-referencing lambda is an ICE** — `go = fn(acc: integer, i: integer) -> integer { … go(…) }` inside a function: *"var_pos underflow in fn 'n___lambda_0': variable 'go' … has no assigned slot"*.
3. **loft#1579 — the cycle refusal's own cure does not work.** The real boundary is wider than recursion: ANY struct or variant field typed `reference<E>` over an ENUM refuses `&x` (*"Cannot assign ref(Shape)["c"] to field SH.s of type ref(Shape)["??"]"*), while a reference to a struct works everywhere and a LOCAL `reference<E>` works. So no recursive struct-enum is buildable the way the compiler suggests, which blocks A5, the recursive forms of C1 and C6, and tier E; the struct-wrapper above is the workaround.

Two more surfaced while correcting the reference pages against these results: **loft#1580** —
`==` on a `value struct` compares identity, not content (DESIGN_DECISIONS C91); **loft#1581** — a
concrete `!=` ignores a user-defined `OpEq`, so `a == b` and `a != b` are both true. Both
`silent-wrong`.

### The document's own claims, corrected

- **Doc bugs 1 and 2 held and are fixed** (`stdlib-interfaces.html` rendered only its heading because the renderer skipped every interface; `09-enum`'s `opposite()` is now a `match`). **3 changed**: D1 is a decided edge, and `25-generics` already states the behaviour and both remedies accurately, so it needs no change. **4 is fixed** ("Match inside an arm"), and `29-match` now also shows the patterns C1, C2 and C4 measured working. **5 was new and is fixed**: `26-closures` said *"A '&' parameter cannot be captured at all, in any shape"*; it now documents (L-CapRef).
- **There is no `loop` keyword** (LUA_BAR's erratum, measured): A8's probe uses `loop { … }`, so rewrite it with `while true` — and note that a `while` GENERATOR runs eagerly on `--native` (COROUTINE.md CL-9).
- **The expectation line cannot be `# bar: …`.** `#` opens a loft annotation (`#rust`, `#cwd`), not a comment. Use `// @BAR: pass` / `// @BAR: refuse "…"` / `// @BAR: measure …`, or reuse the corpus's `@EXPECT_ERROR:` and `@EXPECT_WARNING:` (C7's "a warning counts" is exactly `@EXPECT_WARNING`).
- **A parse failure is a FAIL only when no current spelling expresses the capability.** Rewrite a probe into the existing spelling first (A7, B5, C8, F2 above) — the "syntax is a placeholder" rule cuts both ways.
- **WONTFIX belongs in DESIGN_DECISIONS.md**, the declined-features register. COMPATIBILITY.md is the breaking-change policy; it is the right home only for the separate decision to make a tier a promise.
- **Tier A's order is @PLN165's.** Arc C (several variables, a variable in any parameter — A1–A3, B6), arc D (generic structs and enums — A4–A6), arc E (`map`/`reduce` as library generics). Its STEPS.md already sequences them after the C110 → C126 revision; this file should track that plan, not set a second order.
- **Tier G needs no lifted copies.** Point each row at its `tests/docs/*.loft` file; a copy would be a second home for the same example.


---

## 0. How probes work

Probes live in `tests/bar/ocaml/`, one `.loft` file each, named exactly as the
heading of the entry (`A1_map_two_type_vars.loft`, …). Each file is a complete
program with `fn main()` and `assert(...)` calls.

Each probe carries an expectation on its first line. ⚠ Write it as `// @BAR: …`, not as the
`# bar: …` shown below — `#` opens a file directive in loft (*"Unknown file directive '#bar'"*):

```
# bar: pass
```
The program must compile and run to completion, exit 0, on **both**
`loft --interpret` and `loft --native`. Differential behaviour between backends
is a FAIL even if one side is green.

```
# bar: refuse "substring of the required diagnostic"
```
The compiler must refuse the program, and the diagnostic must contain the
substring. A probe that is *supposed* to be refused and compiles instead is a
FAIL — some entries below measure that loft catches a mistake OCaml would catch.

```
# bar: measure <metric> <bound>
```
The program runs green and additionally reports a number (via the memory
diagnostics or `ticks`) that must satisfy the bound. Only tier E uses this.

A script `tools/bar.loft` (to be written, ~50 lines: iterate the directory,
run both backends, compare with the first line, print a table) turns the
directory into a single `make bar` target. `make bar` must never be part of
`make ci` until the project *chooses* to make a tier a compatibility promise;
until then it is a report, and the report goes into `doc/claude/OCAML_BAR_STATUS.md`
as a dated table.

**Syntax caveat.** Probes for features loft does not have yet use *proposed*
syntax, chosen to be the smallest extension of what exists. The formal rule that
eventually lands may spell it differently. The criterion is the capability
named in the entry's "OCaml expresses" line, never the exact spelling; when the
spelling changes, rewrite the probe and note it in the status table.

**Scoring.** Each tier reports `passed / total`. The bar is reached when every
tier except the ones marked *design-negotiable* is at `total / total`.
Design-negotiable entries are places where loft's memory model gives a defensible
reason to say no forever; they are listed so that "no" is a recorded decision,
not an omission.

---

## Tier A — Parametric polymorphism

The largest gap. Documented state: one type variable per function, it must
appear in the first parameter, `<T, U>` does not parse, generic structs do not
exist (`struct Box<T>` is a parse error). Reference: `25-generics.html`.

### A1_map_two_type_vars

OCaml expresses: `List.map : ('a -> 'b) -> 'a list -> 'b list` — a user-written
`map` whose result element type differs from its input element type.

```
# bar: pass
fn map2<T, U>(v: vector<T>, f: fn(T) -> U) -> vector<U> {
  out: vector<U> = [];
  for e in v { out += [f(e)]; }
  out
}

fn main() {
  lens = map2(["a", "bb", "ccc"], |s| { len(s) });
  assert(lens[2] == 3, "text -> integer: {lens[2]}");
  labels = map2([1, 2], |n| { "#{n}" });
  assert(labels[0] == "#1", "integer -> text: {labels[0]}");
}
```

Measured 2026-09-21: FAIL — `<T, U>` does not parse; @PLN165 arc C.

Documented 2026-09-21 (before measuring): **FAIL** — `<T, U>` does not parse (documented).

### A2_fold_accumulator_type

OCaml expresses: `List.fold_left : ('a -> 'b -> 'a) -> 'a -> 'b list -> 'a`.
Accumulator type independent of element type.

```
# bar: pass
fn fold<T, A>(v: vector<T>, init: A, f: fn(A, T) -> A) -> A {
  acc = init;
  for e in v { acc = f(acc, e); }
  acc
}

fn main() {
  total_len = fold(["ab", "cde"], 0, |acc, s| { acc + len(s) });
  assert(total_len == 5, "fold text into integer: {total_len}");
  joined = fold([1, 2, 3], "", |acc, n| { "{acc}{n}" });
  assert(joined == "123", "fold integer into text: {joined}");
}
```

Measured 2026-09-21: FAIL — arc C.

Documented 2026-09-21 (before measuring): **FAIL** — same root cause as A1.

### A3_type_var_not_in_first_param

OCaml expresses: `let empty () = []` — a polymorphic value whose type is fixed
by the *use site*, and `zip : 'a list -> 'b list -> ('a * 'b) list` where the
second variable first appears in the second argument.

```
# bar: pass
fn zip<T, U>(a: vector<T>, b: vector<U>) -> vector<(T, U)> {
  out: vector<(T, U)> = [];
  for i in 0..len(a) { out += [(a[i], b[i])]; }
  out
}

fn empty_of<T>() -> vector<T> { [] }

fn main() {
  pairs = zip([1, 2], ["x", "y"]);
  assert(pairs[1] == (2, "y"), "zip: {pairs[1]}");
  names: vector<text> = empty_of();
  assert(len(names) == 0, "return-type-directed instantiation");
}
```

Measured 2026-09-21: FAIL — `<T, U>` does not parse; arc C.

Documented 2026-09-21 (before measuring): **FAIL** — "Type variable T must appear in the first
parameter" (documented).

### A4_generic_struct

OCaml expresses: `type ('a, 'b) pair = { first : 'a; second : 'b }`.

```
# bar: pass
struct Pair<T, U> { first: T, second: U }

fn swap<T, U>(p: Pair<T, U>) -> Pair<U, T> {
  Pair { first: p.second, second: p.first }
}

fn main() {
  p = Pair { first: 1, second: "one" };
  q = swap(p);
  assert(q.first == "one", "swapped first: {q.first}");
  assert(q.second == 1, "swapped second: {q.second}");
}
```

Measured 2026-09-21: FAIL — `struct Pair<` does not parse; @PLN165 arc D.

Documented 2026-09-21 (before measuring): **FAIL** — generic structs do not exist (documented).

### A5_generic_recursive_enum

OCaml expresses:
`type 'a tree = Leaf | Node of 'a tree * 'a * 'a tree` and
`let rec size = function Leaf -> 0 | Node (l, _, r) -> 1 + size l + size r`.
A user-defined container, recursive, generic in its element.

```
# bar: pass
enum Tree<T> {
  Leaf,
  Node { left: Tree<T>, value: T, right: Tree<T> },
}

fn size<T>(t: Tree<T>) -> integer {
  match t {
    Leaf => 0,
    Node { left, value, right } => 1 + size(left) + size(right),
  }
}

fn main() {
  t = Node { left: Node { left: Leaf, value: 1, right: Leaf }, value: 2, right: Leaf };
  assert(size(t) == 2, "size: {size(t)}");
  s = Node { left: Leaf, value: "a", right: Leaf };
  assert(size(s) == 1, "instantiated at text: {size(s)}");
}
```

Measured 2026-09-21: FAIL — arc D, and A5b: a direct self-reference is refused naming `reference<Expr>` as the cure, which then refuses an `Expr` value (defect 3).

Documented 2026-09-21 (before measuring): **FAIL** (A4 root cause). Also unverified: whether a
*non-generic* struct-enum may hold its own type directly in a field (not via
`vector<Self>`). If it may not, add probe `A5b_recursive_enum_mono` as a
prerequisite with `enum Expr { Lit { v: integer }, Add { l: Expr, r: Expr } }`.

### A6_user_result_type

OCaml expresses: `type ('a, 'e) result = Ok of 'a | Error of 'e` plus
`Result.bind`. The ability to define `Option`/`Result` as ordinary data, in
user code, without the language's null flow being involved.

```
# bar: pass
enum Result<T, E> {
  Ok { value: T },
  Err { error: E },
}

fn and_then<T, U, E>(r: Result<T, E>, f: fn(T) -> Result<U, E>) -> Result<U, E> {
  match r {
    Ok { value } => f(value),
    Err { error } => Err { error },
  }
}

fn parse_positive(s: text) -> Result<integer, text> {
  n = s as integer;
  if n == null { Err { error: "not a number: {s}" } }
  else if n <= 0 { Err { error: "not positive: {n}" } }
  else { Ok { value: n } }
}

fn main() {
  r = and_then(parse_positive("7"), |n| { Ok { value: "{n * 2}" } });
  assert(r is Ok, "chained ok");
  e = and_then(parse_positive("-1"), |n| { Ok { value: "{n}" } });
  assert(e is Err, "error short-circuits");
}
```

Measured 2026-09-21: FAIL — arcs C + D.

Documented 2026-09-21 (before measuring): **FAIL** (A1 + A4 root causes). Note that `E` appears only
in the enum, never in the function's first parameter directly — A3 must hold.

### A7_user_declared_interface

OCaml expresses: a signature `module type ORDERED = sig type t val compare : t -> t -> int end`
and code parameterised over it. The nearest loft shape is a *user-declared*
interface used as a bound. The catalogue lists "Interfaces & bounded generics"
(@F26); the generics page lists eight built-in bounds and says each guarantees
"exactly the operations in this table"; the stdlib *Interfaces* page is empty.
Whether a user can declare one is therefore **unknown from the docs**.

```
# bar: pass
interface Area {
  fn area(self) -> float;
}

struct Sq { side: float }
struct Circ { r: float }
fn area(self: Sq) -> float { self.side * self.side }
fn area(self: Circ) -> float { 3.0 * self.r * self.r }

fn total_area<T: Area>(v: vector<T>) -> float {
  sum = 0.0;
  for s in v { sum += s.area(); }
  sum
}

fn main() {
  assert(total_area([Sq { side: 2.0 }, Sq { side: 1.0 }]) == 5.0, "bound on user interface");
  assert(total_area([Circ { r: 1.0 }]) == 3.0, "second instantiation");
}
```

Measured 2026-09-21: **PASS** both backends, spelled `fn area(self: Self) -> float`. Doc bug 1 (the empty Interfaces page) stands.

Documented 2026-09-21 (before measuring): **UNKNOWN**. First action: determine the truth, then either
(a) mark PASS and *fill the empty Interfaces reference page*, or (b) mark FAIL.
Either outcome fixes a doc bug.

### A8_associated_type

OCaml expresses: `module type CONTAINER = sig type 'a t val elt : 'a t -> 'a end`
— a signature naming a companion type. The catalogue lists @F113 "Associated
types — an interface names a companion type". Verify it, and that a generic
function can use the companion type in its signature.

```
# bar: pass
interface Source {
  type Item;
  fn next(self) -> Item?;
}

fn drain<S: Source>(s: S) -> vector<S.Item> {
  out: vector<S.Item> = [];
  loop { x = s.next() ?? break; out += [x]; }
  out
}

struct Counter { n: integer, limit: integer }
fn next(self: &Counter) -> integer? {
  if self.n >= self.limit { null } else { self.n += 1; self.n }
}

fn main() {
  got = drain(Counter { n: 0, limit: 3 });
  assert(got == [1, 2, 3], "drained via associated type: {got}");
}
```

Measured 2026-09-21: PARTIAL — the companion works inside a generic (`type Rows: Cursor`, `Self.Rows`, @PLN125); `-> S.Item` in the generic's own signature does not parse (documented).

Documented 2026-09-21 (before measuring): **UNKNOWN** (feature listed, no reference page found).
Depends on A3 (`S.Item` in the return type).

---

## Tier B — Closures

Documented state (`26-closures.html`): capture is automatic; scalars/text are
copied at definition time, collections/structs are shared by reference; a
written scalar may be captured by only one closure; a `&` parameter cannot be
captured; a capturing closure cannot be stored in any collection, only in a
struct field; closures may be returned from functions.

### B1_vector_of_capturing_closures

OCaml expresses: `let handlers = [ (fun x -> x + base); (fun x -> x * scale) ]`.
A list of closures each carrying its own environment.

```
# bar: pass
fn main() {
  base = 10;
  scale = 3;
  steps: vector<fn(integer) -> integer> = [
    |x| { x + base },
    |x| { x * scale },
  ];
  v = 1;
  for f in steps { v = f(v); }
  assert(v == 33, "(1 + 10) * 3 == {v}");
}
```

Measured 2026-09-21: FAIL — named refusal, as documented.

Documented 2026-09-21 (before measuring): **FAIL** — "a capturing closure cannot be stored in a
collection" (documented). This blocks handler tables, behaviour lists,
combinator alternatives and thunk queues; it is the single most consequential
closure limit.

### B2_handler_table

OCaml expresses: `Hashtbl.add on_event "jump" (fun () -> player.vy <- -jump_v)`.
A keyed table of closures that mutate shared state.

```
# bar: pass
struct Player { y: integer, vy: integer }

fn main() {
  p = Player { y: 0, vy: 0 };
  jump_v = 5;
  on: hash<(text, fn())> = {};
  on["jump"] = || { p.vy = -jump_v; };
  on["land"] = || { p.vy = 0; };
  on["jump"]();
  assert(p.vy == -5, "handler mutated shared struct: {p.vy}");
  on["land"]();
  assert(p.vy == 0, "second handler: {p.vy}");
}
```

Measured 2026-09-21: FAIL — B1, and `hash<(…)>` has no tuple element type.

Documented 2026-09-21 (before measuring): **FAIL** (B1 root cause). Keyed-collection syntax for a
`(text, fn())` entry is illustrative.

### B3_two_closures_share_written_scalar  *(design-negotiable)*

OCaml expresses: `let r = ref 0 in (fun () -> incr r), (fun () -> !r)` — two
closures over one mutable cell. loft's documented rule: a scalar a closure
writes to may be captured by only one closure; the documented cure is a struct.

```
# bar: pass
fn main() {
  count = 0;
  bump = || { count += 1; };
  read = || { count };
  bump(); bump();
  assert(read() == 2, "both closures see the same cell: {read()}");
}
```

Measured 2026-09-21: FAIL — named refusal (*"captured by 2 closures"*).

Documented 2026-09-21 (before measuring): **FAIL** (documented refusal). Negotiable because the
struct workaround is one line and the restriction has a clear no-GC rationale.
If the decision is "never", record it in DESIGN_DECISIONS.md and mark this entry
`WONTFIX` rather than deleting it.

### B4_recursive_local_function

OCaml expresses: `let rec go acc = function [] -> acc | x :: xs -> go (x + acc) xs in go 0 lst`
— a recursive helper defined inside a function body, closing over locals.

```
# bar: pass
fn sum_to(n: integer) -> integer {
  step = 1;
  fn go(acc: integer, i: integer) -> integer {
    if i > n { acc } else { go(acc + i, i + step) }
  }
  go(0, 1)
}

fn main() {
  assert(sum_to(10) == 55, "local recursive fn closing over n and step: {sum_to(10)}");
}
```

Measured 2026-09-21: FAIL — *"'fn' definitions must be at file scope"*; the self-referencing lambda is an ICE (defect 2).

Documented 2026-09-21 (before measuring): **UNKNOWN** — no local `fn` or self-referencing lambda
appears in the reference. If refused, the criterion can also be met by a
self-referencing lambda; either spelling passes.

### B5_closure_captures_ref_param  *(design-negotiable)*

OCaml has no `&`; the analogue is capturing a mutable record passed in. loft
refuses capturing a `&` parameter in any shape (documented). Record the
decision; probe kept so the refusal is *tested*, not assumed.

```
# bar: refuse "cannot be captured"
fn bump_all(v: &vector<integer>) {
  f = |i| { v[i] += 1; };
  for i in 0..len(v) { f(i); }
}
fn main() { xs = [1]; bump_all(&xs); }
```

Measured 2026-09-21: **works** — capture and write-through succeed on both backends, so the expectation flips to `pass` and 26-closures is stale (doc bug 5). The probe as written was invalid (`bump_all(&xs)`, untyped `|i|`).

Documented 2026-09-21 (before measuring): **PASS as a refusal** (documented). Flip to `# bar: pass`
only if the design changes.

### B6_compose  *(depends on A1, A3)*

OCaml expresses: `let ( >> ) f g x = g (f x)`.

```
# bar: pass
fn compose<T, U, V>(f: fn(T) -> U, g: fn(U) -> V) -> fn(T) -> V {
  |x| { g(f(x)) }
}

fn main() {
  h = compose(|n| { n + 1 }, |n| { "{n}!" });
  assert(h(1) == "2!", "composed: {h(1)}");
}
```

Measured 2026-09-21: FAIL — arc C.

Documented 2026-09-21 (before measuring): **FAIL** (A1/A3 root causes; also `V` appears only in the
second parameter and the return type).

---

## Tier C — Pattern depth and exhaustiveness

Documented state (`29-match.html`, `09-enum.html`): arms bind struct-enum
fields, but "the variable names must match the field names exactly"; tuple
patterns are shown over scalars and `_`; guards, or-patterns, ranges, `null`
and enum exhaustiveness exist; a scalar match with no matching arm answers null
silently; the "nested match" section is a match *expression* in an arm body,
not a nested pattern.

### C1_nested_constructor_pattern

OCaml expresses:
`match e with Add (Lit 0, r) -> r | Add (l, Lit 0) -> l | e -> e`.

```
# bar: pass
enum Expr {
  Lit { v: integer },
  Add { l: Expr, r: Expr },
}

fn simplify(e: Expr) -> Expr {
  match e {
    Add { l: Lit { v: 0 }, r } => r,
    Add { l, r: Lit { v: 0 } } => l,
    _ => e,
  }
}

fn main() {
  e = Add { l: Lit { v: 0 }, r: Lit { v: 7 } };
  assert(simplify(e) is Lit, "left zero dropped");
  match simplify(e) { Lit { v } => assert(v == 7, "kept 7: {v}"), _ => assert(false, "wrong shape") }
}
```

Measured 2026-09-21: **PASS** over a non-recursive nesting, both backends; this recursive form is blocked on A5b.

Documented 2026-09-21 (before measuring): **FAIL** — no nested constructor in field position exists.
Prerequisite: a struct-enum field of its own enum type (see A5b note).

### C2_literal_in_field_position

OCaml expresses: `| Circle 0.0 -> "point"`.

```
# bar: pass
enum Shape { Circle { r: float }, Rect { w: float, h: float } }

fn main() {
  s = Circle { r: 0.0 };
  k = match s {
    Circle { r: 0.0 } => "point",
    Circle { r }      => "circle {r}",
    Rect { w, h }     => "rect",
  };
  assert(k == "point", "literal inside field pattern: {k}");
}
```

Measured 2026-09-21: **PASS** both backends.

Documented 2026-09-21 (before measuring): **FAIL** (documented: bindings only).

### C3_field_rename_and_as_binding

OCaml expresses: `| Rect { w = width; _ } as whole -> ...` — binding a field
under a different name, and binding the whole value alongside its parts.

```
# bar: pass
enum Shape { Circle { r: float }, Rect { w: float, h: float } }

fn main() {
  s = Rect { w: 2.0, h: 3.0 };
  d = match s {
    whole @ Rect { w: width, h } => "{width}x{h} of {whole.w}",
    Circle { r } => "c",
  };
  assert(d == "2x3 of 2", "rename + as-binding: {d}");
}
```

Measured 2026-09-21: FAIL — neither the rename nor `whole @` parses.

Documented 2026-09-21 (before measuring): **FAIL** (documented: names must equal field names).

### C4_tuple_of_variants

OCaml expresses:
`match state, ev with Playing p, Damage d when p.hp <= d -> Dead | ...`.
Two scrutinees, each destructured, in one arm.

```
# bar: pass
enum State { Dead, Playing { hp: integer } }
enum Event { Tick, Damage { amount: integer } }

fn step(s: State, e: Event) -> State {
  match (s, e) {
    (Dead, _) => Dead,
    (Playing { hp }, Damage { amount }) if hp <= amount => Dead,
    (Playing { hp }, Damage { amount }) => Playing { hp: hp - amount },
    (Playing { hp }, Tick) => Playing { hp },
  }
}

fn main() {
  s = step(Playing { hp: 3 }, Damage { amount: 5 });
  assert(s is Dead, "lethal damage");
  t = step(Playing { hp: 3 }, Damage { amount: 1 });
  match t { Playing { hp } => assert(hp == 2, "hp: {hp}"), _ => assert(false, "wrong") }
}
```

Measured 2026-09-21: **PASS** both backends in the real spelling — the probe's arm body `Playing { hp }` uses field-init shorthand, which loft does not have; `Playing { hp: hp }` passes every assertion.

Documented 2026-09-21 (before measuring): **FAIL** (tuple elements documented as scalars/`_` only).
Note @F122 multiple dispatch covers the *dispatch* half of this; it does not
give one arm access to both sides' fields plus a guard over both.

### C5_or_pattern_with_bindings

OCaml expresses: `| Circle r | Sphere r -> r` when both sides bind the same
names at the same types.

```
# bar: pass
enum Solid { Circle { r: float }, Sphere { r: float }, Cube { s: float } }

fn main() {
  x = Sphere { r: 2.0 };
  r = match x {
    Circle { r } | Sphere { r } => r,
    Cube { s } => s,
  };
  assert(r == 2.0, "shared binding across or-pattern: {r}");
}
```

Measured 2026-09-21: FAIL — does not parse.

Documented 2026-09-21 (before measuring): **UNKNOWN** — or-patterns are documented only over bare
variants and scalars.

### C6_exhaustiveness_through_nesting

OCaml *warns* here: leaving out `Node (Leaf, _, Node _)` is reported as a
missing case with the pattern spelled out.

```
# bar: refuse "not covered"
enum T { Leaf, Node { l: T, r: T } }

fn f(t: T) -> integer {
  match t {
    Leaf => 0,
    Node { l: Leaf, r: Leaf } => 1,
    Node { l: Node { l, r }, r: _ } => 2,
  }
}
fn main() { f(Leaf); }
```

Measured 2026-09-21: PARTIAL — non-recursively nested, the hole is refused but named by its outer variant (*"missing: W"*); this recursive form is blocked on A5b.

Documented 2026-09-21 (before measuring): **FAIL** (depends on C1; once nested patterns exist, the
checker must see through them — this probe ensures the two land together).

### C7_scalar_match_hole_is_diagnosed

OCaml refuses to be silent: a non-exhaustive match is a warning by default.
loft documents that a scalar match selecting no arm answers null and "the
compiler will not remind you". A *warning* (not an error — null-flow is a
deliberate design) that names the hole satisfies this entry.

```
# bar: refuse "may select no arm"
fn main() {
  k = match 7 {
    1 => "one",
    2 => "two",
  };
  assert(k == null, "silent null");
}
```

Measured 2026-09-21: FAIL — silent on both backends.

Documented 2026-09-21 (before measuring): **FAIL** (documented silence). Treat `refuse` here as
"emits a diagnostic containing the substring"; a warning that still compiles
counts, and `bar.loft` should accept warnings for this entry.

### C8_vector_head_tail_pattern

OCaml expresses: `| [] -> 0 | x :: xs -> x + sum xs`. loft's @F99 sequence
patterns (alternation, optionals, repetition, capture) may already cover this.

```
# bar: pass
fn sum(v: vector<integer>) -> integer {
  match v {
    [] => 0,
    [x, ..rest] => x + sum(rest),
  }
}
fn main() { assert(sum([1, 2, 3]) == 6, "head/tail: {sum([1,2,3])}"); }
```

Measured 2026-09-21: PASS with a `_` arm (the probe's `sum` is a reserved name); `[]` + `[x, ..rest]` is not seen as total.

Documented 2026-09-21 (before measuring): **UNKNOWN** — verify against the @F99 syntax and rewrite the
probe in it if it differs.

---

## Tier D — Nullability soundness

OCaml's `option` is checked: a `'a`-typed value is never absent. loft documents
two holes where a declared non-null type carries null or a fabricated default.

### D1_generic_return_cannot_smuggle_null

Documented: `gen_v[0]` is typed `T` and still answers null on an empty vector,
"so the null travels through a return type that says it cannot be there".

```
# bar: refuse "may be null"
fn first<T>(v: vector<T>) -> T { v[0] }
fn main() {
  e: vector<integer> = [];
  x = first(e);
  assert(x == 7, "unreachable");
}
```

Measured 2026-09-21: FAIL — a decided edge: (N-Index) trusts a constant index by contract (C80, @PLN102 D1).

Documented 2026-09-21 (before measuring): **FAIL**. Acceptable fixes: (a) constant-index reads are
typed `T?` like computed ones; (b) a returned expression whose static type is
`T?` cannot satisfy a declared `T` without `??` or `?`; (c) a flow proof of
`len(v) > 0`. Any of the three passes.

### D2_empty_stub_default_is_explicit

Documented: an empty-body variant method "hands back the type's default", a
real value a caller cannot distinguish from a computed one.

```
# bar: refuse "empty body"
enum Shape { Circle { r: float }, Rect { w: float, h: float } }
fn area(self: Circle) -> float { 3.0 * self.r * self.r }
fn area(self: Rect) -> float { }
fn main() {
  s: Shape = Rect { w: 1.0, h: 1.0 };
  total = s.area() + 1.0;
  assert(total == 1.0, "silent 0.0");
}
```

Measured 2026-09-21: FAIL — silent `0.0` on both backends.

Documented 2026-09-21 (before measuring): **FAIL**. A warning at the *read site* (the result is used)
satisfies this; an unused stub may stay silent.

---

## Tier E — Structural sharing  *(design-negotiable as a tier)*

OCaml's persistent structures cost O(path) per update because immutable nodes
are shared. loft's copy/move semantics (@F106) and store make sharing an
explicit, non-default operation. This tier does not ask for a GC; it asks
whether *some* opt-in mechanism (a refcounted immutable node type, a
`shared<T>`, store-level aliasing) can give persistent update at the same
asymptotic cost. If the answer is "no, by design", record it and mark the tier
WONTFIX. These probes use the memory diagnostics (`stdlib-memory-diagnostics`);
adapt the metric call to whatever it exposes.

### E1_persistent_cons_is_O1

OCaml: `let ys = 0 :: xs` allocates one cell regardless of `len xs`.

```
# bar: measure bytes_delta < 256
enum List { Nil, Cons { head: integer, tail: List } }

fn main() {
  xs: List = Nil;
  for i in 0..10000 { xs = Cons { head: i, tail: xs }; }
  before = memory_used();
  ys = Cons { head: -1, tail: xs };
  after = memory_used();
  println("bytes_delta {after - before}");
  assert(ys is Cons, "built");
  assert(xs is Cons, "original still intact");
}
```

Measured 2026-09-21: BLOCKED on A5b; `memory_used()` does not exist — use `store_memory()`'s record count.

Documented 2026-09-21 (before measuring): **UNKNOWN** (recursive enum prerequisite; metric name
illustrative).

### E2_persistent_tree_insert_is_Olog

OCaml: `Map.add` into a map of n entries allocates O(log n) nodes and leaves the
old map valid.

```
# bar: measure bytes_delta < 4096
# (a 100k-element balanced tree; one insert must not copy the tree)
```

Body: a user-written balanced tree of 100k integers (any balancing scheme),
one `insert` producing a *new root* while the *old root* remains a valid tree
with 100k elements; report `bytes_delta` across the single insert; assert both
roots answer `contains` correctly for a probe value. Write the body once A5 and
C1 pass; until then this entry is blocked and reads `BLOCKED(A5,C1)`.

Measured 2026-09-21: BLOCKED on A5b.

Documented 2026-09-21 (before measuring): **BLOCKED**.

---

## Tier F — Dispatch exhaustiveness

Documented (`09-enum.html`): a per-variant method called on a value held as the
enum "works like a match on its variant, and a match must cover every variant";
a method on the enum itself is the `_` arm. @F122 adds multiple dispatch. This
tier checks the OCaml-grade property — a missing case is a compile error that
*names* it — survives the move from single to multiple dispatch.

### F1_missing_combination_named

```
# bar: refuse "Rock, Paper"
enum Hand { Rock, Paper, Scissors }
fn beats(a: Rock, b: Scissors) -> boolean { true }
fn beats(a: Paper, b: Rock) -> boolean { true }
fn beats(a: Scissors, b: Paper) -> boolean { true }
fn main() {
  x: Hand = Rock; y: Hand = Paper;
  assert(!beats(x, y), "no definition for (Rock, Paper)");
}
```

Measured 2026-09-21: PARTIAL — refused, naming the declared set rather than the missing pair.

Documented 2026-09-21 (before measuring): **UNKNOWN** — verify the diagnostic exists and names the
pair (either order of the two names is acceptable in the substring check;
adjust the expectation to the real wording once known, but it must name both).

### F2_enum_level_fallback_covers_combinations

```
# bar: pass
enum Hand { Rock, Paper, Scissors }
fn beats(a: Rock, b: Scissors) -> boolean { true }
fn beats(a: Paper, b: Rock) -> boolean { true }
fn beats(a: Scissors, b: Paper) -> boolean { true }
fn beats(a: Hand, b: Hand) -> boolean { false }
fn main() {
  x: Hand = Rock; y: Hand = Paper;
  assert(!beats(x, y), "fallback picked");
  assert(beats(Paper, Rock), "specific picked over fallback");
}
```

Measured 2026-09-21: FAIL — defect 1: on a plain enum all nine combinations take the fallback; struct-enum variants dispatch correctly.

Documented 2026-09-21 (before measuring): **UNKNOWN**.

---

## Tier G — Regression floor (documented PASS, keep green)

These are things the docs already show working. They are in the bar so that
work on tiers A–F cannot regress them unnoticed. Each is a straight lift of a
documented example; do not "improve" them.

| probe | source page | what it pins |
|---|---|---|
| G1_escaping_closure | 26-closures | `make_adder(10)` returned lambda keeps its capture |
| G2_closure_into_map_filter | 26-closures | capturing lambda handed to `map`/`filter` |
| G3_struct_field_holds_closure | 26-closures | `Stepper { advance: fn … }` |
| G4_single_T_bounded_user_type | 25-generics | `gen_max<T: Ordered>(Money, Money)` via `OpLt` |
| G5_walkable_tree_walk | 25-generics | `Crate` + `children` → `tree_walk` |
| G6_enum_exhaustiveness_error | 29-match | `# bar: refuse` — omitted variant is named |
| G7_guard_reads_binding | 29-match | `Circle { radius } if radius > 10` |
| G8_or_range_null_text_patterns | 29-match | the scalar pattern set |
| G9_tuple_scalar_pattern | 29-match | `(2, "b")` |
| G10_yield_from | 27-coroutines | generator delegation |
| G11_sequence_pattern | @F99 page | alternation / repetition / capture |
| G12_multiple_dispatch_basic | @F122 page | one name, two-parameter combination |
| G13_enum_fallback_method | 09-enum | `fn area(self: Shape)` as the `_` arm |

Measured 2026-09-21: covered — each cited page is generated from a `tests/docs/*.loft` file that `make ci` runs.

---

## Documentation bugs found while measuring

File these regardless of any implementation decision.

1. `stdlib-interfaces.html` renders only its heading. Either the generator
   dropped the body or the page was never written. It is the page A7/A8 need.
2. `09-enum.html` implements `opposite()` with an if-chain where the page's own
   later section shows `match`; the example teaches the pattern the page argues
   against. Rewrite with `match`.
3. `25-generics.html` states the `T`-typed `v[0]` null leak plainly and offers
   only "check the length before calling". That is a documented soundness hole
   (D1); the doc should link to the tracking issue.
4. `29-match.html` "Nested match" heading describes nested *expressions*. Rename
   to "Match inside an arm" so that "nested patterns" is not a term the docs
   appear to already own.

---

## Working rules for the agent

- Re-measure before believing this file. Each entry's `Measured <date>:` line is
  the last measurement; replace it with today's when you run the probes again. The
  probe files and `make bar` are the baseline PR still owed — "OCAML_BAR baseline",
  no feature work in it.
- One probe file per entry, first line is the expectation, nothing else about
  the harness leaks into the probe.
- When a probe's proposed syntax cannot even be parsed, that is still FAIL, not
  "invalid probe". The probe is a request; the syntax is a placeholder.
- When a decision is taken that an entry is out of scope by design, do not
  delete it. Mark `WONTFIX`, cite its DESIGN_DECISIONS.md entry (the declined-features
  register), keep the probe as a `refuse` so the refusal stays tested.
- A tier only becomes part of `make ci` by an explicit decision recorded in
  COMPATIBILITY.md. Until then `make bar` is a report.
- Never mark PASS on one backend. Both or neither.
- Tier A is @PLN165 (flexible generics): arc C gives A1–A3 and B6, arc D gives A4–A6,
  arc E gives `map`/`reduce` as library generics. Follow its STEPS.md order; it already
  puts several-variable *functions* before generic *types*, which is the order this entry
  used to ask for.
