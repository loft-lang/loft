# OCAML_BAR — expressiveness probes measured against OCaml

**Status of this document:** a bar, not a plan. It lists things OCaml expresses
directly, and whether loft does. Every entry is a small program that either runs green on both
backends or is refused with a named diagnostic. Nothing here is a commitment to
implement; it is a ruler the project can be held against, and re-measured after
every change that touches generics, closures, patterns, dispatch or nullability.

**Audience:** the coding agent. Read this whole file before touching any probe.

**Method.** Every probe is run on both backends and scored by the CAPABILITY its "OCaml
expresses" line names, in loft's real spelling. § Evaluation is the one home of each entry's
status; an entry below holds only its probe and the question it asks.  Where the docs and the
compiler disagree, the compiler is the truth and the doc page is a bug to file.  Earlier
measurements, and the documentation-derived statuses they replaced, are in
[OCAML_BAR-history.md](OCAML_BAR-history.md).

---

## Evaluation

Measured at `b194678c` (origin/main), every probe on both backends, from a scratch directory. <!-- doc-lint: ok -->
**The two backends agreed on every cell except where a cell says otherwise.**  A probe whose
spelling is wrong for a capability loft has is scored in the real spelling, and the row names
it.

| entry | measured | what answered |
|---|---|---|
| A1 map, two variables | **PASS** | `fn map2<T, U>(…)` as written |
| A2 fold | **PASS** | as written |
| A3 variable not in first param | FAIL | `zip<T, U>` building a `vector<(T, U)>`: the interpreter skips `main`, panics or segfaults, native fails `E0308` — [loft#1868](https://github.com/loft-lang/loft/issues/1868) (`silent-wrong`); a generic struct element (`vector<Two<T>>`) works.  `empty_of<T>() -> vector<T>` is refused: *"Generic function must have at least one parameter of type T"* <!-- doc-lint: ok --> |
| A4 generic struct | **PASS** | as written |
| A5 generic recursive enum | FAIL | `Node { left: reference<Tree<T> >, … }` is refused (*"variant Node has no field 'left'"*).  `>>` closing two type-argument lists in a variant field is read as comparison operators; write `> >` |
| A5b recursive enum, monomorphic | **PASS** | `Add { l: reference<Expr>, r: reference<Expr> }`, each node bound to a local and linked with `&`; nesting the constructors inline is refused (*"Cannot assign ref(Lit) to field Add.l"*) |
| A6 user `Result` | **PASS** | two spellings fixed: field-init shorthand does not exist (`Err { error: error }`), and a text parse is discharged at the cast (`s as integer ?? 0`) |
| A7 user-declared interface | **PASS** | the spelling is `fn area(self: Self) -> float` (INTERFACES.md) |
| A8 associated type | PARTIAL | the companion works inside a generic (`type Rows: Cursor`, `Self.Rows`, @PLN125); naming it in the generic's OWN signature (`-> vector<S.Item>`) does not parse, as INTERFACES.md documents |
| B1 closures in a vector | FAIL | named refusal: *"a capturing closure cannot be stored in a collection …"* |
| B2 handler table | FAIL | B1, and `hash<(…)>` has no tuple element type |
| B3 two closures, one written scalar | FAIL | named refusal: *"mutated through a closure and captured by 2 closures"* — design-negotiable |
| B4 local recursive function | FAIL | `fn` only at file scope; the self-referencing lambda is refused naming the cure (*"'go' is not bound yet … declare a file-scope 'fn go(…)'"*) |
| B5 capture a `&` parameter | **PASS** | capturing and writing through a `&` parameter works (`[1,10]` → `[2,11]`), written `fn(i: integer) { v[i] += 1; }` |
| B6 compose | **PASS** | `compose<T, U, V>` returning `\|x\| { g(f(x)) }`, called with `fn(n: integer) -> integer { … }` lambdas or named functions.  An untyped `\|n\|` argument is not inferred through a generic parameter (*"No matching operator '+' on 'T'"*) |
| C1 nested constructor pattern | PARTIAL | non-recursive nesting PASS; the recursive shape through a `reference<Expr>` field never matches on the interpreter and fails `E0605` on native — [loft#1870](https://github.com/loft-lang/loft/issues/1870) (`silent-wrong`) <!-- doc-lint: ok --> |
| C2 literal in field position | **PASS** | `Circle { r: 0.0 } => "point"` |
| C3 field rename + as-binding | PARTIAL | rename PASS (`Rect { w: width, h }`); `whole @ Rect {…}` → *"'whole' is not a variant"* |
| C4 tuple of variants | **PASS** | as written once the arm BODY spells `Playing { hp: hp }` (no field-init shorthand) |
| C5 or-pattern with bindings | **PASS** | the spelling is `,`: `Circle { r }, Sphere { r } => r` (`@FR-P-Multi`, LOFT_CONTROL.md § Match expressions).  `\|` joins variant names only; between patterns that bind it is refused (*"Expect token =>"*) |
| C6 exhaustiveness through nesting | PARTIAL | the hole is refused, named by its outer variant (*"missing: X"*), not as `X { i: B }` |
| C7 scalar-match hole diagnosed | PARTIAL | a warning, phrased as the null the hole produces (*"a nullable `text?` is stored into the return value"*), not naming the uncovered value |
| C8 head/tail pattern | PASS with `_` | `[]` + `[x, ..rest]` is not seen as total (*"a slice pattern can fail …"*); the probe's `fn sum` collides with the reserved stdlib name |
| D1 generic return smuggles null | FAIL — a DECIDED edge | `formal/types.md` (N-Index) trusts a constant index by contract (C80) |
| D2 empty-stub default | FAIL | silent `0.0`, both backends |
| E1, E2 structural sharing | not measured | `memory_used()` does not exist; a recursive enum links nodes bound to locals (A5b), so the probes need a new design before they measure sharing |
| F1 missing combination named | changed | a definition over a plain-enum VALUE is refused at the declaration (*"'Rock' is a value of the plain enum 'Hand', not a type … take a 'Hand' and 'match' on its value"*) |
| F2 enum-level fallback | **PASS** | `match (a, b) { (Rock, Scissors) => true, …, _ => false }` over the plain enum |
| G1–G13 regression floor | covered | every cited page is a test: `tests/docs/*.loft` generate the reference pages and run in `make ci` |

**Score (capability, both backends):** A 5/8 + 1 partial (and A5b's monomorphic form passes) · B 2/6 ·
C 4/8 + 4 partial · D 0/2 · E not measured · F 1/2.

### Open defects

| entry | issue | what it is |
|---|---|---|
| A3 | [loft#1868](https://github.com/loft-lang/loft/issues/1868) | a tuple holding a type variable, as a vector element, is never instantiated — `silent-wrong` <!-- doc-lint: ok --> |
| C1 | [loft#1870](https://github.com/loft-lang/loft/issues/1870) | a field sub-pattern through a `reference<T>` field never matches — `silent-wrong` <!-- doc-lint: ok --> |

### Spellings the probes got wrong

A probe that does not parse is a FAIL only when no current spelling expresses the capability.
Rewrite it first:

- **No field-init shorthand** in an expression: `Playing { hp: hp }`, not `Playing { hp }`.
  It exists in a PATTERN.
- **One arm for several variants with bindings is `,`**, not `|`.
- **A typed lambda is `fn(n: integer) -> integer { … }`**; `|n: integer|` is refused.
- **`> >`** closes two type-argument lists in a variant field; `>>` there reads as two comparisons.
- **There is no `loop` keyword** — `while true`.
- **The expectation line is `// @BAR: …`** — `#` opens a file directive.
- **A stdlib name is reserved** (`sum`).
- **A text parse is discharged at the cast**: `s as integer ?? 0`.

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
until then it is a report, and the report goes into `doc/claude/bars/OCAML_BAR_STATUS.md`
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

Several type variables, generic structs and generic enums exist (@PLN165); § Evaluation
names what is still refused. Reference: `25-generics.html`.

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

---

## Working rules for the agent

- Re-measure before believing this file. § Evaluation is the last measurement and names
  its commit; when you run the probes again, move it to OCAML_BAR-history.md and write the
  new one. The probe files and `make bar` are the baseline PR still owed — "OCAML_BAR
  baseline", no feature work in it.
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
- Tier A's generics are @PLN165's (finished). What remains in tier A is a defect
  (loft#1868) or a refusal § Evaluation names, not a plan arc. <!-- doc-lint: ok -->
