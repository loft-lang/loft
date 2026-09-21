# LUA_BAR — what Lua gets from having no declarations, measured against loft

**Status of this document:** a bar, not a plan. Companion to [OCAML_BAR.md](OCAML_BAR.md), same
harness, different question. OCAML_BAR asks what the type system can say; this file asks what
Lua's absence of declarations buys a game scripter, and whether loft has a typed answer whose
use site is as short. Nothing here is a commitment to implement.

**Audience:** the coding agent. Read the whole file, then OCAML_BAR.md § 0, before touching any
probe.

**Method.** Inspected 2026-09-21 from the repository itself (`doc/*.html` as rendered pages,
`doc/features/*.md`, `doc/claude/COROUTINE.md`, `doc/claude/INTERFACES.md`,
`doc/claude/DESIGN_DECISIONS.md`, `doc/claude/SANDBOX.md`). Statuses are what those files state.
First job of the agent: re-measure, replace every `Status 2026-09-21:` line, commit the
baseline with no feature work in the same PR.

**What this bar is not.** It does not count total lines. On an algorithm over declared types
loft is already as compact as Lua or shorter (comprehensions, filtered `for`, loop attributes,
`is` with capture, `?? return`, `match` as an expression, named arguments). A probe that would
only document that win is not here; tier I pins a few wins so they cannot regress, nothing more.

**That measurement has been taken** (§ Evaluation, below): every entry carries a
`Measured 2026-09-21:` line above the documentation-derived one, and where the measurement
contradicts a rule in this file, the Evaluation's corrections win.

---

## Evaluation — measured 2026-09-21

Every probe with a program was run on both backends against the loft3 tree
(`tuxedo-1562-layout-gate`, origin/main plus the loft#1572 fix), from a scratch directory, each run
inside a cgroup memory cap. **The cap is not optional for this tier:** LB3 and LB4 run away on
`--native` (below), and two uncapped runs of them took the measuring session down through the
OOM killer — `LOFT_TIMEOUT` bounds time, not memory. Run such a probe as its own capped unit:
`systemd-run --user --unit=<name> -p MemoryMax=2G -p MemorySwapMax=0 -p RuntimeMaxSec=90 …`.
No probe file or harness is committed yet.

| entry | measured | what answered |
|---|---|---|
| LA1 closed set | **PASS** both | one-line use site |
| LA2 interface-typed element | FAIL | *"'Actor' is a Interface, not a type — the element of vector<T> must be a struct or enum"* — the recorded design (INTERFACES.md: constraints, not types) |
| LA3 mixed literal | FAIL | LA2's root cause |
| LB1 generators in a vector | FAIL — **loft#1585** | *"Fatal: cannot build this record — its type never resolved"*; a struct field holding one fails too. No scheduler can be written, so tier B's scheduler half is blocked |
| LB2 cutscene | interpreter PASS · native FAIL | after the first `next` the trace already reads `hello;hurt;walk;` — the documented CL-9 eager path (a yield inside `if`, a statement after a yield) |
| LB3 per-tick value | interpreter PASS · native **runs away** | the `while` loop is eager on native and `dt` is 0 when it runs, so it never ends: OOM-killed at the 2 GB cap |
| LB4 infinite generator | interpreter PASS · native **runs away** | OOM-killed at the 2 GB cap. `while` is not covered by CL-9 slice 1; `for _ in 0..2000000000 { …; yield n; }` IS lazy (`1 2`). COROUTINE.md's CL-9 row omitted `while` — corrected |
| LB5 abandon frees | **PASS** both | |
| LC1 action table of fn-refs | **PASS** both | real keyed form: `struct Action { name: text, f: fn(Player) }` in `hash<Action[name]>`, use site `(actions[cmd] ?? noop).f(p)` — one line |
| LC2 with captures | FAIL | named refusal: *"collection of a struct type that holds a capturing closure is not supported"* (OCAML_BAR B1's root) |
| LC3 function by runtime name | FAIL | no `fn_named`; and calling a parenthesised fn value, `(f ?? g)(p)`, does not parse |
| LC4 variadics | PARTIAL | refused at parse time (*"Syntax error: unexpected '..'"*), no crash — but it names no alternative |
| LD1 type-directed inner literal | **PASS** both | `spawns: [{ x: 1, y: 2, kind: Bat }, …]` works — the entry expected FAIL |
| LD2 level as a module | **PASS** both | `use cave; lvl = cave();` with the data behind a zero-argument function; a struct-valued `pub const` is refused with that cure named |
| LD3 JSON asset | interpreter PASS · native FAIL — **loft#1584** | `lvl = Level.parse(src) ?? return;` does not build on native: any `x = <record> ?? return` reads an undeclared variable. The probe's JSON also needs `{{`/`}}` — a loft string is a format string |
| LD4 defaults | **PASS** both | `advice[omitted-field-zero]` on `boss`, as intended |
| LE1 tuple destructure | **PASS** both | one-line use site |
| LF1 operator overloading | **PASS** both | |
| LF2 default on miss | budget met, no declared form | `colours["enemy"].v ?? "grey"` at the use site, both backends; no declaration-level fallback exists (a decision still to record) |
| LF3 intercept writes | validation half **PASS** both | the proxy half remains a decision to record |
| LG1, LG2 | not measured | manual by definition |
| LH1–LH3 | not measured | the entry's own rule requires a named reference machine and browser |
| LI1 typo field | **PASS** | *"Unknown field P.postion — did you mean 'position'?"* |
| LI2 nil call | not expressible as written | `(fn(integer) -> integer)?` reads the parentheses as a tuple type |
| LI3 overflow is null | **PASS** both | |
| LI4 enum typo | **PASS** | refused (*"Unknown field Kind.Bta"* — it says field where it means variant) |
| LI5 par loop | **PASS** both | `for a in src par(b = sq(a), 2) { … }` |

**Score (capability, both backends):** A 1/3 · B 1/5 (three more pass on the interpreter only)
· C 1/4 + 1 partial · D 3/4 (LD3 interpreter only) · E 1/1 · F 3/3 · I 4/5 · G, H not measured.

### Defects the measurement surfaced (filed)

- **loft#1583** — a lazily compiled loop generator with a `boolean` local does not build on
  `--native` (the coroutine state field is `u8`, the body assigns a `bool`). Found while bounding LB4.
- **loft#1584** — `x = <nullable record> ?? return` does not build on `--native` (LD3).
- **loft#1585** — a generator cannot be held in a vector or a struct field, and the refusal reads as
  a compiler internal (LB1; `needs-design`).

The `while`-generator runaway (LB3, LB4) is CL-9's open slice 3, not a new defect; what was wrong
was its documentation, which listed the eager shapes without `while`. COROUTINE.md, the Coroutines
page and a new Safety-page trap now name it and its memory consequence.

### The document's own claims, corrected

- **All five doc bugs are fixed:** `00-vs-python` (operator overloading, `while true`) and the same
  loop claim on `00-vs-rust`; INTERFACES.md's stale associated-types line; the empty
  `stdlib-interfaces.html` (the renderer skipped every interface); and the CL-9 trap, now on the
  Safety page.
- **Erratum 3 to OCAML_BAR holds for A8 only.** There is no `loop` keyword (`while true` is the
  form, measured); OCAML_BAR's B3 does not use `loop`.
- **A keyed collection is `hash<S[key]>` over a struct with a key field**, never `hash<(text, …)>`;
  LC1, LC2 and LF2 were measured in that form.
- **OCAML_BAR § 0's corrections apply here too:** the first line is `// @BAR: …`, and a parse
  failure is a FAIL only where no current spelling expresses the capability.

---

## 0. Harness additions

Everything in OCAML_BAR.md § 0 applies. Two additions:

**Use-site budget.** Most probes here have a declaration part (paid once) and a use site (paid
every time a scripter writes the pattern). The criterion is that the use site is no longer than
the Lua reference shown in the entry. Mark it in the probe:

```
// use-site begin
for e in entities { e.update(dt); e.draw(); }
// use-site end
```

and state the budget on the first line:

```
# bar: pass budget 1
```

`bar.loft` counts non-blank, non-comment lines between the markers and fails the probe when the
count exceeds the budget. Declarations outside the markers are free. A probe may have several
marked regions; the budget is their sum.

**Manual probes.** Tier G (live reload) cannot be driven from a probe file alone. `# bar:
manual` marks an entry whose pass/fail is recorded by a human or by the debugger's scripted RPC
(@F51) and pasted into the status table with the date. `bar.loft` lists manual probes as MANUAL
and never fails on them.

Probes live in `tests/bar/lua/`. Files are named as the headings below.

---

## Tier A — Open heterogeneous sets

The Lua game loop is one line over a table holding anything with `update` and `draw`, including
kinds defined in a library the author never opened.

Documented state: struct-enums with per-variant methods give this for a closed set the author
declares (`09-enum.html`). INTERFACES.md § "What is out of scope" says interface values —
`x: Ordered = my_priority` — are not supported: "interfaces are constraint annotations, not
types", and adds "all of the above can be added later".

### LA1_closed_set_loop (regression floor)

Lua reference (use site, 1 line):

```lua
for _, e in ipairs(entities) do e:update(dt); e:draw() end
```

```
# bar: pass budget 1
enum Entity {
  Player { x: float, vx: float },
  Bat    { x: float, dir: float },
}
fn update(self: Player, dt: float) { self.x += self.vx * dt; }
fn update(self: Bat, dt: float)    { self.x += self.dir * 20.0 * dt; }
fn draw(self: Player) -> text { "P@{self.x}" }
fn draw(self: Bat) -> text    { "B@{self.x}" }

fn main() {
  entities: vector<Entity> = [Player { x: 0.0, vx: 10.0 }, Bat { x: 5.0, dir: -1.0 }];
  dt = 0.5;
  out = "";
  // use-site begin
  for e in entities { e.update(dt); out += e.draw(); }
  // use-site end
  assert(out == "P@5B@-5", "closed set dispatch: {out}");
}
```

Measured 2026-09-21: **PASS** both backends; use site one line.

Documented 2026-09-21 (before measuring): PASS (documented). Pins that the closed-set answer is already at Lua's budget.

### LA2_interface_typed_element

Lua reference (use site, 2 lines): a library file adds a kind, the game only appends an
instance.

```lua
table.insert(entities, ghostlib.new_ghost(3))
for _, e in ipairs(entities) do e:update(dt); e:draw() end
```

```
# bar: pass budget 2
interface Actor {
  fn update(self: Self, dt: float)
  fn draw(self: Self) -> text
}
struct Player { x: float }
fn update(self: Player, dt: float) { self.x += dt; }
fn draw(self: Player) -> text { "P" }

// pretend this struct lives in a library the game did not write
struct Ghost { t: float }
fn update(self: Ghost, dt: float) { self.t += dt; }
fn draw(self: Ghost) -> text { "G" }

fn main() {
  entities: vector<Actor> = [Player { x: 0.0 }];
  out = "";
  // use-site begin
  entities += [Ghost { t: 0.0 }];
  for e in entities { e.update(1.0); out += e.draw(); }
  // use-site end
  assert(out == "PG", "open set through interface values: {out}");
}
```

Measured 2026-09-21: FAIL — an interface is not a type (the recorded design).

Documented 2026-09-21 (before measuring): FAIL — interface values are out of scope by the recorded design (static
dispatch only). This is the widest single gap against Lua for game code. Two acceptable answers,
either passes: (a) interface-typed elements with a per-element dispatch table, monomorphised per
interface rather than per type; (b) an open enum a library can extend (`extend enum Entity {
Ghost { … } }`) so the game's `vector<Entity>` accepts a library kind without the game author
editing the enum. If neither is wanted, mark WONTFIX with the COMPATIBILITY.md paragraph; do not
delete.

### LA3_mixed_vector_literal

Lua reference (1 line): `local hud = { score_label, health_bar, minimap }` — three unrelated
kinds in one list, drawn in order.

```
# bar: pass budget 1
interface Widget { fn draw(self: Self) -> text }
struct Label { s: text }
struct Bar   { v: integer }
struct Map   { w: integer }
fn draw(self: Label) -> text { self.s }
fn draw(self: Bar) -> text   { "[{self.v}]" }
fn draw(self: Map) -> text   { "map{self.w}" }

fn main() {
  // use-site begin
  hud: vector<Widget> = [Label { s: "hi" }, Bar { v: 3 }, Map { w: 8 }];
  // use-site end
  out = "";
  for w in hud { out += w.draw(); }
  assert(out == "hi[3]map8", "mixed literal: {out}");
}
```

Measured 2026-09-21: FAIL — LA2's root cause.

Documented 2026-09-21 (before measuring): FAIL (LA2 root cause). Today this requires an enum wrapping the three, which
is fine for the author's own types and is what LA1 pins; the entry exists because a HUD is the
commonest place a scripter mixes kinds from several libraries.

---

## Tier B — Coroutines as a scheduler's unit

Lua: a behaviour is a function with `coroutine.yield()` between frames; a scheduler holds a
table of live coroutines and resumes each per tick, passing `dt` in and receiving status out.

Documented state (`27-coroutines.html`, COROUTINE.md CL-9): generators are values of type
`iterator<T>`, advanced by `next(gen)`; parameters (including a shared struct) survive yields;
`yield from` delegates; an abandoned generator is freed. On `--native` only a loop whose body
ends in one unconditional yield of a scalar or text is lazy; a yield inside `if`/`match`, more
than one yield per iteration, a statement after the yield, a nested loop, or a `continue` still
runs the whole loop eagerly, so side effects differ between backends (slices 2–4 of CL-9 are
open). `next` takes no value in. A generator has no return.

### LB1_generators_in_a_vector

Lua reference (use site, 3 lines):

```lua
table.insert(tasks, coroutine.create(patrol))
for i = #tasks, 1, -1 do
  if coroutine.status(tasks[i]) == "dead" then table.remove(tasks, i) else coroutine.resume(tasks[i]) end
end
```

```
# bar: pass budget 3
fn patrol(id: integer, n: integer) -> iterator<integer> {
  for step in 0..n { yield id * 100 + step; }
}

fn main() {
  tasks: vector<iterator<integer>> = [];
  log = "";
  // use-site begin
  tasks += [patrol(1, 2), patrol(2, 3)];
  for round in 0..4 {
    for t in tasks { v = next(t); if v != null { log += "{v} "; } }
  }
  // use-site end
  assert(log == "100 200 101 201 202 ", "round-robin over live generators: {log}");
}
```

Measured 2026-09-21: FAIL — loft#1585: a generator cannot be stored in a vector or a struct field.

Documented 2026-09-21 (before measuring): UNKNOWN — no page shows an `iterator<T>` stored in a collection; the closure
rule ("a capturing closure cannot be stored in a collection") may or may not extend to a
suspended generator frame. If this fails, no scheduler can be written and the whole tier is
blocked; measure it first.

### LB2_cutscene_shape_is_lazy_on_native

Lua reference: the shape scripters actually write.

```lua
function cutscene()
  say("hello")          ; coroutine.yield()
  if player.hp < 5 then say("you look hurt") ; coroutine.yield() end
  walk_to(door)         ; coroutine.yield()
  say("bye")
end
```

```
# bar: pass
struct Trace { said: text }

fn cutscene(hurt: boolean, t: Trace) -> iterator<integer> {
  t.said += "hello;";
  yield 1;
  if hurt {
    t.said += "hurt;";
    yield 2;
  }
  t.said += "walk;";
  yield 3;
  t.said += "bye;";
}

fn main() {
  t = Trace { said: "" };
  g = cutscene(true, t);
  next(g);
  assert(t.said == "hello;", "after first resume only the first line ran: {t.said}");
  next(g);
  assert(t.said == "hello;hurt;", "conditional yield resumed lazily: {t.said}");
  next(g);
  assert(t.said == "hello;hurt;walk;", "third slice: {t.said}");
  next(g);
  assert(t.said == "hello;hurt;walk;bye;", "tail after last yield ran on the final advance: {t.said}");
}
```

Measured 2026-09-21: interpreter PASS; native FAIL (eager — trace `hello;hurt;walk;` after the first `next`).

Documented 2026-09-21 (before measuring): FAIL on `--native` (documented: a yield inside `if` and a statement after a
yield take the eager path). This is CL-9 slices 2 and 3. This probe is the Lua scheduler test:
until it passes on both backends, a cutscene script behaves differently in the browser than in
the interpreter.

### LB3_per_tick_value_reaches_the_generator

Lua reference (use site, 1 line in the scheduler, 1 in the behaviour):

```lua
coroutine.resume(co, dt)          -- scheduler
local dt = coroutine.yield()      -- behaviour
```

loft has no value-in channel on `next`. Two acceptable spellings: a shared struct captured at
creation (documented pattern; what the probe uses), or a `next(g, value)` form if one is ever
added. Either passes; the budget is 2.

```
# bar: pass budget 2
struct Clock { dt: float }

fn mover(c: Clock, dist: float) -> iterator<float> {
  x = 0.0;
  while x < dist {
    x += 10.0 * c.dt;
    yield x;
  }
}

fn main() {
  clock = Clock { dt: 0.0 };
  g = mover(clock, 25.0);
  // use-site begin
  clock.dt = 1.0; a = next(g);
  clock.dt = 2.0; b = next(g);
  // use-site end
  assert(a == 10.0 && b == 30.0, "generator saw each tick's dt: {a} {b}");
}
```

Measured 2026-09-21: interpreter PASS; native runs away — OOM-killed at a 2 GB cap. Run only as a capped unit.

Documented 2026-09-21 (before measuring): UNKNOWN — the shared-struct pattern is documented for reads (`Trace.steps`);
the `while` loop with a statement before the yield is a CL-9 eager shape on native, so this
probably fails there for the same reason as LB2. Record separately: it is the design-endorsed
spelling and must work.

### LB4_infinite_generator_is_lazy

Lua reference: `while true do … coroutine.yield() end` is the default shape of every behaviour.

```
# bar: pass
fn blink() -> iterator<boolean> {
  on = false;
  while true {
    on = !on;
    yield on;
  }
}

fn main() {
  g = blink();
  seen = "";
  for i in 0..5 { seen += if next(g) ?? false { "1" } else { "0" }; }
  assert(seen == "10101", "five advances of an infinite generator: {seen}");
}
```

Measured 2026-09-21: interpreter PASS; native runs away — OOM-killed at a 2 GB cap (`while` is eager; a `for` over a large range is lazy). Run only as a capped unit.

Documented 2026-09-21 (before measuring): FAIL on `--native` (CL-9 slice 3: `while`/`loop` and a statement before the
yield). The interpreter passes. Note: the reference pages avoid `while true` — `00-vs-python.html`
says loft has no infinite-loop form and shows `for _ in 0..2147483647`; whether `while true` or a
`loop` keyword exists today is itself something to verify and document in one place.

### LB5_abandon_frees (regression floor)

```
# bar: pass
fn many() -> iterator<integer> { for i in 0..1000000 { yield i; } }
fn main() {
  taken = 0;
  for v in many() { taken += 1; if v >= 2 { break; } }
  assert(taken == 3, "broke early: {taken}");
}
```

Measured 2026-09-21: **PASS** both backends.

Documented 2026-09-21 (before measuring): PASS (documented).

---

## Tier C — Dispatch on a runtime name

Lua: `actions[name](e)`, `_G["on_" .. event]`, `obj[method]()`.

Documented state: fn-refs are first-class (@F23); a keyed collection may hold non-capturing
lambdas only (`26-closures.html`); `type_named("Row")` looks a type up by runtime name (@F107);
no function-by-name lookup is documented; variadic functions are a recorded non-goal
(DESIGN_DECISIONS C100, TUPLES.md).

### LC1_action_table_of_fn_refs (regression floor)

Lua reference (use site, 1 line): `actions[cmd](player)`

```
# bar: pass budget 1
struct Player { y: integer }
fn jump(p: Player) { p.y += 5; }
fn duck(p: Player) { p.y -= 1; }

fn main() {
  actions: hash<(text, fn(Player))> = {};
  actions["jump"] = jump;
  actions["duck"] = duck;
  p = Player { y: 0 };
  cmd = "jump";
  // use-site begin
  (actions[cmd] ?? duck)(p);
  // use-site end
  assert(p.y == 5, "dispatched by runtime string: {p.y}");
}
```

Measured 2026-09-21: **PASS** both backends in the real keyed form (`hash<Action[name]>`); use site one line.

Documented 2026-09-21 (before measuring): PASS expected (non-capturing fn-refs in a keyed collection are documented).
Keyed-collection entry syntax is illustrative; rewrite to the real `hash<T[keys]>` form.

### LC2_action_table_with_captures

Same use site; the handlers close over game state. Cross-reference OCAML_BAR.md B1/B2 — one root
cause, listed here because it is the Lua-shaped consequence.

```
# bar: pass budget 1
struct World { score: integer }
fn main() {
  w = World { score: 0 };
  bonus = 10;
  actions: hash<(text, fn())> = {};
  actions["coin"] = || { w.score += bonus; };
  actions["hit"]  = || { w.score -= bonus / 2; };
  ev = "coin";
  // use-site begin
  (actions[ev] ?? || {})();
  // use-site end
  assert(w.score == 10, "capturing handler from table: {w.score}");
}
```

Measured 2026-09-21: FAIL — named refusal (a collection of a closure-holding struct).

Documented 2026-09-21 (before measuring): FAIL (documented refusal).

### LC3_function_by_runtime_name (design-negotiable)

Lua reference (1 line): `_G["on_" .. event](e)`

`type_named` exists for types. A `fn_named("on_jump")` returning `fn(Player)?` would be the
analogue. The native backend's dead-code elimination and the sandbox admission set both have a
legitimate interest in refusing this. Record the decision; if declined, LC1 is the documented
replacement and this entry becomes a refuse pinning the diagnostic.

```
# bar: pass budget 1
struct Player { y: integer }
fn on_jump(p: Player) { p.y += 5; }
fn main() {
  p = Player { y: 0 };
  event = "jump";
  // use-site begin
  (fn_named("on_{event}") ?? on_jump)(p);
  // use-site end
  assert(p.y == 5, "looked up by name: {p.y}");
}
```

Measured 2026-09-21: FAIL — no `fn_named`; `(f ?? g)(p)` does not parse either.

Documented 2026-09-21 (before measuring): UNKNOWN (no such lookup documented).

### LC4_no_variadics_is_recorded (WONTFIX, pinned)

DESIGN_DECISIONS C100: loft has no variadic functions; the format string is the single
string-building tool. This probe pins that the refusal is a diagnostic, not a parse crash, and
that it names the alternative.

```
# bar: refuse "format string"
fn log(fmt: text, ...args) { }
fn main() { log("x={} y={}", 1, 2); }
```

Measured 2026-09-21: PARTIAL — a parse-time refusal, no crash, but no alternative named.

Documented 2026-09-21 (before measuring): UNKNOWN — the decision is recorded; whether the compiler says so when someone
writes it is not.

---

## Tier D — Data as literals

Lua: a level file is `return { name = "Cave", spawns = { {x=1,y=2,kind="bat"} } }`; no type is
named anywhere and code reads fields.

Documented state: struct literals name their type (`Point { x: … }`); a field may carry
`= default` and an omitted field takes its type's zero (@F12); a bare variant name is inferred
from context (@F15); `Type.parse` reads JSON into a declared type (@F42); a type receives the
parts of a format string (@F94). Nothing shows an inner struct literal without its type name.

### LD1_type_directed_inner_literal

Lua reference (use site, the whole literal, 4 lines):

```lua
return { name = "Cave", spawns = {
  { x = 1, y = 2, kind = "bat" },
  { x = 4, y = 1, kind = "rat" },
} }
```

```
# bar: pass budget 4
enum Kind { Bat, Rat }
struct Spawn { x: integer, y: integer, kind: Kind }
struct Level { name: text, spawns: vector<Spawn> }

fn main() {
  // use-site begin
  lvl = Level { name: "Cave", spawns: [
    { x: 1, y: 2, kind: Bat },
    { x: 4, y: 1, kind: Rat },
  ] };
  // use-site end
  assert(len(lvl.spawns) == 2 && lvl.spawns[1].kind == Rat, "inner literals typed by the field");
}
```

Measured 2026-09-21: **PASS** both backends — the entry expected FAIL.

Documented 2026-09-21 (before measuring): FAIL expected — every documented inner literal names its struct. The enum half
(`Bat` without `Kind.`) is documented as @F15; this entry asks for the same rule on struct
literals where the field type fixes them. This is the entry that decides whether a loft level
file reads like a Lua one once the three declarations are paid.

### LD2_level_file_is_a_loft_module

Lua reference (1 line): `local lvl = dofile("levels/cave.lua")`

```
# bar: pass budget 1
// file: tests/bar/lua/levels/cave.loft
//   pub const CAVE = Level { name: "Cave", spawns: [ Spawn { x: 1, y: 2, kind: Bat } ] };
// (the Level/Spawn/Kind declarations live in a shared module both files `use`)
use levels.cave;
fn main() {
  // use-site begin
  lvl = CAVE;
  // use-site end
  assert(lvl.name == "Cave", "level as a module-level constant: {lvl.name}");
}
```

Measured 2026-09-21: **PASS** both backends via `use cave; lvl = cave();`; a struct-valued `pub const` is refused, naming that cure.

Documented 2026-09-21 (before measuring): UNKNOWN — module-level `pub const` of a struct value and the `use` path form
need verifying against `17-libraries.html`. Note the honest difference to pin in the entry's
notes: a Lua `dofile` is a runtime load of an arbitrary path; loft admits scripts at mod-load
(SANDBOX.md S8: no `run_script` boundary, by design). So the loft answer is compile-time or a
JSON asset via LD3, never a runtime `.loft` load. That is a recorded decision, not a gap.

### LD3_json_asset_to_typed_struct (regression floor)

Lua reference (1 line): `local lvl = json.decode(read("cave.json"))`

```
# bar: pass budget 1
enum Kind { Bat, Rat }
struct Spawn { x: integer, y: integer, kind: Kind }
struct Level { name: text, spawns: vector<Spawn> }

fn main() {
  src = `{"name":"Cave","spawns":[{"x":1,"y":2,"kind":"Bat"}]}`;
  // use-site begin
  lvl = Level.parse(src) ?? return;
  // use-site end
  assert(lvl.spawns[0].kind == Bat, "typed parse: {lvl.spawns[0].kind}");
}
```

Measured 2026-09-21: interpreter PASS; native FAIL — loft#1584 (`?? return` on a record).

Documented 2026-09-21 (before measuring): PASS expected (@F42). Pins that the checked answer to Lua's unchecked
`json.decode` costs the same at the use site.

### LD4_defaults_and_omitted_fields (regression floor)

Lua: a missing key is nil and code guards it. loft: `= default`.

```
# bar: pass budget 1
struct Spawn { x: integer, y: integer, hp: integer = 3, boss: boolean }
fn main() {
  // use-site begin
  s = Spawn { x: 1, y: 2 };
  // use-site end
  assert(s.hp == 3 && !s.boss, "defaults: {s.hp} {s.boss}");
}
```

Measured 2026-09-21: **PASS** both backends.

Documented 2026-09-21 (before measuring): PASS (documented).

---

## Tier E — Multiple returns

Lua: `local x, y = pos()`. loft: tuples (@F11, `28-tuples.html`).

### LE1_tuple_return_destructure (regression floor)

```
# bar: pass budget 1
fn pos() -> (integer, integer) { (3, 4) }
fn main() {
  // use-site begin
  (x, y) = pos();
  // use-site end
  assert(x + y == 7, "destructured tuple: {x} {y}");
}
```

Measured 2026-09-21: **PASS** both backends; use site one line.

Documented 2026-09-21 (before measuring): PASS expected — verify the destructuring assignment form against
`28-tuples.html`; if only `t = pos(); t.0` exists, this is a 2-line use site and the entry FAILs
its budget.

---

## Tier F — Metatables

Lua's metatables carry three things games use: operator overloading, `__index` (prototype
fallback / default on miss), and `__newindex` (intercept writes). Documented state: operator
methods `OpAdd`/`OpEq`/`OpLt`/`OpIndex` (`25-generics.html`, INTERFACES.md measured table);
`computed(expr)` fields and `assert(...)` field constraints (@F12); `OpDrop` (@F115); lazy store
binding for a collection miss (@F108). No general "intercept field access".

### LF1_operator_overloading (regression floor)

```
# bar: pass budget 1
struct V2 { x: float, y: float }
fn OpAdd(self: V2, o: V2) -> V2 { V2 { x: self.x + o.x, y: self.y + o.y } }
fn OpMul(self: V2, k: float) -> V2 { V2 { x: self.x * k, y: self.y * k } }
fn main() {
  a = V2 { x: 1.0, y: 2.0 }; b = V2 { x: 3.0, y: 4.0 };
  // use-site begin
  c = (a + b) * 0.5;
  // use-site end
  assert(c.x == 2.0 && c.y == 3.0, "vector arithmetic: {c.x} {c.y}");
}
```

Measured 2026-09-21: **PASS** both backends.

Documented 2026-09-21 (before measuring): PASS (documented). Pinned mainly because `00-vs-python.html` still says loft
has "no operator overloading" — see doc bugs.

### LF2_default_on_miss (design-negotiable)

Lua reference: `setmetatable(cfg, { __index = defaults })` — a lookup that misses falls through
to another table.

loft's typed answer for records is `= default` (LD4). The remaining Lua case is a keyed
collection that answers a fallback for a missing key without the caller writing `?? default` at
every site. @F108 does this for a store-backed collection on a miss; the entry asks whether an
in-memory hash can declare a fallback once.

```
# bar: pass budget 1
fn main() {
  colours: hash<(text, text)> = {} default "grey";
  colours["player"] = "blue";
  // use-site begin
  c = colours["enemy"];
  // use-site end
  assert(c == "grey", "declared fallback on miss: {c}");
}
```

Measured 2026-09-21: budget met at the use site (`?? "grey"`), both backends; no declared fallback exists.

Documented 2026-09-21 (before measuring): UNKNOWN (probably FAIL; `?? "grey"` at the use site is the documented spelling
and is a 1-line use site too, so the budget is met either way — the entry exists to record
whether the fallback belongs on the declaration).

### LF3_intercept_writes (WONTFIX candidate, pinned)

Lua reference: `__newindex` to validate or log every write. loft's answer is `assert(...)` on the
field (validation) and nothing for logging/proxying. Record the decision.

```
# bar: pass budget 1
struct Stats { hp: integer assert($.hp >= 0 && $.hp <= 100) }
fn main() {
  s = Stats { hp: 50 };
  // use-site begin
  s.hp = 80;
  // use-site end
  assert(s.hp == 80, "validated write: {s.hp}");
}
```

Measured 2026-09-21: validation half **PASS** both backends.

Documented 2026-09-21 (before measuring): PASS expected for the validation half (@F12 assert); the proxy half is out of
scope and should be written down as such.

---

## Tier G — Hot reload and live editing (manual)

Lua: `dofile` in a REPL while the game runs. loft: `LOFT_LIVE_RELOAD=1` swaps edited functions
into the running program keeping its state (@F56); `loft debug prog.loft:12` stops at a line and
lets you read and edit the live frame (@F51).

### LG1_function_swap_keeps_state

```
# bar: manual
Procedure: run tests/bar/lua/LG1_game.loft under LOFT_LIVE_RELOAD=1 on the
interpreter; let the score counter reach 50; edit `fn bonus() -> integer { 1 }`
to `{ 10 }` and save; the next tick adds 10 and the score is still >= 50.
Then repeat with --native and record whether reload is supported there at all.
```

Measured 2026-09-21: not measured (manual).

Documented 2026-09-21 (before measuring): PASS expected on `--interpret` (@F56); `--native` behaviour unknown — record it.

### LG2_live_frame_edit

```
# bar: manual
Procedure: `loft debug LG1_game.loft:<tick line>`; at the breakpoint set the
score variable to 999 through the debugger; resume; the drawn HUD shows 999.
```

Measured 2026-09-21: not measured (manual).

Documented 2026-09-21 (before measuring): PASS expected (@F51).

---

## Tier H — Runtime footprint (measured)

Lua's VM is roughly 250 KB and starts in well under a millisecond; a browser game cares about
bytes transferred and time to first frame. loft's claim of no GC pauses is also a number, not a
sentence. Reference program for all three: `tests/bar/lua/LH_sprite.loft` — a window, one sprite
moving, 600 frames, the smallest thing that exercises the graphics library. Record the machine
and browser in the status table; compare against the previous baseline, not against Lua, once a
baseline exists. The Lua-derived bounds below are the bar; the first measurement will likely be
above some of them.

### LH1_html_bundle_size

```
# bar: measure gzip_bytes < 1000000
Procedure: `loft --html LH_sprite.loft`, then sum the gzip -9 size of every
file the page loads (wasm, js, html, the content pack). Report gzip_bytes.
Stretch bound: 500000.
```

Measured 2026-09-21: not measured (needs a named reference machine and browser).

Documented 2026-09-21 (before measuring): UNKNOWN.

### LH2_time_to_first_frame

```
# bar: measure ms < 500
Procedure: headless browser, cold cache, from navigation start to the first
`requestAnimationFrame` callback that drew the sprite; median of 5 runs.
Also record `--interpret` start-to-first-print of hello.loft (bound 50 ms).
```

Measured 2026-09-21: not measured (needs a named reference machine and browser).

Documented 2026-09-21 (before measuring): UNKNOWN.

### LH3_frame_time_jitter

```
# bar: measure p99_over_median < 2.0
Procedure: 600 frames in the browser build; collect per-frame time; report
99th percentile divided by median. Lua+GC typically lands at 3–6 with a
naive allocator pattern; loft's no-GC design should hold under 2.
```

Measured 2026-09-21: not measured (needs a named reference machine and browser).

Documented 2026-09-21 (before measuring): UNKNOWN. This is the one where loft should beat Lua outright; measure it so
the claim has a number.

---

## Tier I — Pinned wins

Each is a refuse or a pass that Lua cannot offer. Kept small.

| probe | expectation | what it pins |
|---|---|---|
| LI1_typo_field_is_compile_error | `# bar: refuse "no field"` | `p.postion` fails to compile; Lua indexes nil at runtime |
| LI2_nil_call_is_compile_error | `# bar: refuse` | calling a `fn(...)?` without `??` is refused; Lua's "attempt to call a nil value" |
| LI3_overflow_is_null_not_wrap | `# bar: pass` | integer overflow yields null, `?? 0` recovers; Lua 5.3 wraps silently |
| LI4_enum_typo_is_compile_error | `# bar: refuse` | `Kind.Bta` refused; a Lua string kind is any string |
| LI5_par_loop | `# bar: pass` | `par(...)` over a vector; stock Lua has no threads |

Measured 2026-09-21: LI1, LI3, LI4, LI5 PASS (LI3 and LI5 on both backends); LI2 is not expressible as written.

Documented 2026-09-21 (before measuring): PASS (documented) for all; confirm by running.

---

## Documentation bugs found while measuring

1. `00-vs-python.html` says structs have "no operator overloading (`__add__`, `__eq__`)".
   `25-generics.html` and INTERFACES.md document `OpAdd`, `OpEq`, `OpLt`, `OpMul`, `OpIndex`. The
   comparison page is hand-checked and has drifted.
2. `00-vs-python.html` says there is no infinite-loop form and recommends `for _ in
   0..2147483647`. Verify whether `while true` (or a `loop` keyword) exists; document one answer
   in one place, and fix the comparison page.
3. INTERFACES.md § "What is out of scope" lists associated types as unsupported; its later
   "measured" table says associated types are yes (@PLN125 arc A). The section is stale. Same
   file, two answers.
4. `stdlib-interfaces.html` is empty (also reported in OCAML_BAR.md). Given INTERFACES.md, that
   page should show the interface declaration form.
5. `27-coroutines.html` describes CL-9 in a ⚠ paragraph as a caveat. It is a backend-divergence
   in observable behaviour and belongs in `23-safety.html` as a named trap, not only in a
   coroutines footnote.

## Errata to OCAML_BAR.md, from this inspection

- A7_user_declared_interface: INTERFACES.md status is "implemented (I1–I9)"; user interfaces
  exist. Expected to flip to PASS on measurement. The empty reference page remains a bug.
- A8_associated_type: `type Rows: Cursor` in an interface body and `Self.Rows` in signatures are
  reported as landed. Rewrite the probe in that spelling (`Self.Item`, not `S.Item`); A3 remains a
  prerequisite for the return type.
- A8 and B3 use a `loop { … }` form that may not exist (see doc bug 2). Rewrite with `while true`
  or `for` once the loop form is known.

## Working rules for the agent

- Same rules as OCAML_BAR.md: re-measure first, baseline PR with no feature work, both backends
  or neither, WONTFIX is a recorded decision with a citation and the probe kept as a refuse,
  `make bar` is a report until a tier is promoted by an explicit COMPATIBILITY.md entry.
- Measure LB1 before anything else in tier B; if generators cannot be held in a vector the tier
  is blocked and the root cause is shared with OCAML_BAR B1.
- Tier H needs a fixed reference machine and browser named in the status table; a number
  without that is not a measurement.
- Budgets are set by the Lua reference in the entry. Do not lower a budget to make a probe pass;
  if the Lua reference is wrong, fix the reference and say so in the status table.
- Root-cause order: LB2/LB4 (CL-9 slices 2–3) and LA2 are the two items that change what kind of
  game can be written in loft; everything else in this file is polish by comparison.
