<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 strand 1b — reading the ports beside their homes: three fixes, designed

A child document of [1b-interpreter.md](1b-interpreter.md).  Status: **R-2s built through step 9
(owner, 2026-10-30: "implement our plans for the tree") — on by default, the rules in
`formal/`, @C159, suite cells under `L-OutOfLine`, `T-Recursive`, `B-OutOfLine`,
`F-Render-Child`; step 10 (the port) built — the IR image's own children are children.
R-2's `e!` built too (owner, 2026-10-30: "build the ! desugar for ?? panic too"): `(N-Present)`,
@C160, and every `?? panic` of the ports is `!`.  R-3 built (owner, 2026-10-31: "do the
recommendation") — `lost-write` judges each field of a copy.  R-1 built (owner, 2026-10-31: "now
do R-1 too") — `Own` an enum, the "none" answers `integer?`, one visitor per traversal; what it
still owes is § What follows R-1.**

## Why (owner, 2026-10-30)

*"Does your current loft side resemble the rustc side?  Is it clear to read versus the rustc
code?"* — and then *"we are evaluating, so create designs for the fixes."*  The loft ports of
B-1/B-2 (`selfhost/src/own.loft` 1754 lines, `binds.loft` 959, `lower.loft` 2644) follow their
Rust homes function for function: each function keeps its Rust name and cites its home
(`file:line`), the arms of `generate_set` keep their order and labels, and the bind trace checks
the correspondence row by row.  They read worse than their homes in three places, measured
below.  Each design starts from the rule or decision that already speaks, as
[1b-reference.md](1b-reference.md) orders every finding.

## R-1 — the oracle's lattice and the "none" answers as loft types

**Measured.**  `own.loft` encodes `Own` as one integer — `Borrowed{b}` = b, `Join{b}` =
65536 + b, Owned −1, Unknown −2, `NOT_YET` 65534 — and every "no variable" answer of
`own.loft` and `binds.loft` is −1 or 65535.  One `walk(mode)` stands in for the several Rust
traversals `any_node`/`for_each_child` callers write with closures.  Nothing in the language
forced either: loft has payload enums and `integer?`.

**Rule / decision.**  None is touched: the representation is the port's own choice, and goal
constraint 5 asks for the reference's decisions, not its spelling.  The binding constraint is
the comparison: the `ownership` and `binds` layers must stay EQUAL.

**Design.**
- `enum Own { Owned, Unknown, NotYet, Borrowed { base: integer }, Join { base: integer } }` with
  `base_of`, `is_join`, `join` as functions over it — the Rust `Own`'s variants, `NOT_YET` a
  variant rather than a reserved base.  `fmt_own`/`render_own` match on it.
- Every fact answering `Option<u16>` answers `integer?` (null for `None`): `entry_witness`,
  `owner_witness`, `displaced_buffer_witness`, `projection_container_var`,
  `record_copy_source`, `callee_of`, `heap_def_nr`.  Callers test `!= null` where they test
  `>= 0` today.  `u16::MAX` stays a value where the Rust keeps it as one (`base == u16::MAX`,
  a join with no single base).
- `walk(mode)` splits into one function per Rust traversal it serves, each named after it.

**Verification.**  A pure refactor: the `ownership` layer on the kernels, the 60-program
sample and every `tests/scripts` program the front end accepts, the `binds` layer on the
kernels and the sample, and the census — all EQUAL / unchanged before and after.  The
interpreter's speed on the kernels is measured and reported (an enum is a record in loft; the
integer was free), never a gate.

**Cost.**  Mechanical: about 300 changed lines, in one commit per file.

**Built.**  All three parts, with one correction to the design: `NOT_YET` stays the reserved
`Borrowed { base: u16::MAX - 1 }` (`own::not_yet()`), NOT a variant.  The oracle relies on the
identity — a call whose argument classifies `NOT_YET` answers a borrow of that base, which is
`NOT_YET` again where it is joined — and the variant changed three verdicts.  Measured as a pure
refactor, against the loft side's own outputs before it (stricter than EQUAL with the Rust):
the ported oracle's ownership report on every image of the kernels and `tests/scripts` (521),
and on each the runner's stdout, stderr, exit, bind trace and allocator trace — identical in
the interpreter (a 46-image sample) and in the C build (all 521, once loft#2295 was fixed — before,
three programs read a released store there).  The compiler's diagnostics on both
tools are unchanged.  The kernels run no slower (0.28 s before, 0.22 s after, three runs each).

## What follows R-1 — the log to the best port code

R-1 met compiler defects; the port is written in its best form and a defect is fixed in the
compiler, never routed around in the port.  What remains, in order:

1. **`run.loft` `sub_text`** — `t[from..till]` with nullable bounds, the port's last build
   warning.  The language answers an empty slice for a null end and reads a null start as 0
   where `(N-Store)` says a null bound reads null (loft#2306); once that is fixed the warning is
   the store's own and `sub_text`'s comment ("a null `till` takes the one character at `from`")
   is rewritten to the rule.
2. **`lower.loft` `Rotation`** keeps four `-1` fields where its Rust (`codegen.rs:7290`) has two
   `Option` pairs: one `(integer, integer)?` field per pair.
3. Then the plan's queue (handoff): loft#2260, E-4, F-1.

**Done (2026-10-31).** Every "none" answer of `own`, `binds`, `lower` and `types` is `integer?`
— the `Option` facts of the Rust, the lookups of the port's own, the local `match … _ => -1`
bindings — and `binds`' second spellings of `u16::MAX`/`u32::MAX` are `own::MAX`/`own::UMAX`.
A `let Some(x) = … else { return … }` is a single-name guard, so `(N-Guard)` proves `x` after it
and no `!` follows; a compound guard is split into one guard per fact, and a `!` stays only
after a compound test, which proves nothing.  `crt::key_float` passes `is_null` as its two
float siblings do.  Measured each step: all 521 images, C build, same compiler — identical;
tests/selfhost.rs passes.

Fixed on the way: loft#2292 (a local renamed onto the return buffer was copied onto itself — a
zeroed record on every backend); loft#2295 (a hoisted call argument a borrowing return answers
was freed before the return — a use-after-free in the shared IR the C runtime exposed first, and
the reason the `_`-prefixed visitor parameters seemed to move it: they moved the allocation
order); and a closure-record leak R-2s S1(b) had introduced on this branch (`___oldrec` bound as
a view of its slot again).  With loft#2295 fixed, the R-1 port's C build is identical to the
baseline on all 521 images, no workaround in the port.  loft#2298 (the port's `--native` build
failed: a discharged local compared in an `else if` — a block argument to a `#rust` template is
now bound first), loft#2300 (that discharge, consumed in place, leaked a copy of the record) and
loft#2293 (a value-record local was also tracked as a witnessed owner on --native: E0308).
Then, from the `integer?` pass: loft#2302 (a heap field's not-null marker rode into the next
operand, so a live `??` default was reported redundant — and `loft fix` deleted it), loft#2303
(pass 1 read `x == null` on a heap or enum value with the truthy polarity: a guard proved nothing
after it) and loft#2305 (pass 1 read a call to a function declared below as a `null` arm, so an
inferred local joined to `τ?`).

## R-2 — the `?? panic(…)` discharges: mostly one representation choice

**Measured.**  258 discharges across the three files.  By the index they discharge:

| shape | own | binds | lower | what it is |
|---|---:|---:|---:|---|
| a constant index (`inner[0]`, `args[0]`) | 50 | 16 | 110 | almost all the IR schema's BOX-OF-ONE: `ir.loft` holds a single child as `vector<Node>` of one (`NdSet { inner }`, `NdBlock { block }`), because loft has no owned single-child field and `reference<T>` would triple the width (the schema's own comment) |
| a variable index | 25 | 16 | 40 | a definition, variable or type by number — the image's tables, present by construction |
| `len(…) - 1` | 3 | 1 | 5 | the last operator of a block, after a length test |

**Rule / decision.**  `(N-Index)`: `v[i]` types `τ?`, and `(N-Domain)` already elides the `?`
when the index is PROVABLY in range — a constant, a range, a guard.  `@C89` declined Rust's
mitigation layer (`unwrap`, `if let`) — the layer TUPLE variants force; it did not decline sugar
inside the `τ?`-with-`??` family, which already has `x?` (`(N-Default)`, @PLN116): *"`??` = the
default YOU give, `?` = the default the TYPE gives."*  The owner (2026-10-30): *"I am not against
syntactic sugar here, the ? operator already returns the type default instead of constantly
writing ?? 0."*

**Design, in four parts — the sugar first, then the shapes that remain:**
- **`e!` — the discharge with no default: present, or a halt that names it.**  The family's
  third member: `??` the default you give, `?` the type's default, `!` none — it must be there.
  - *Rule* (types.md, beside `(N-Default)`): `(N-Present)  Γ ⊢ e ⇒ τ? ⟹ Γ ⊢ e! ⇒ τ`, with
    `e!  ≡  e ?? panic("<e> is null")`, the message the parser writes from the source text and
    the panic carrying the position, as every panic does.  No new runtime: it IS the idiom the
    ports spell by hand, so a production run that logs a panic and continues continues as `??
    panic` does today.  No `has_default` side condition, so it reaches what `?` cannot: a
    reference (`(D-NoRef)`) and a bare enum (`(D-NoEnumF)`) — the ports' `Node`, `Block` and
    `Definition`, which is why they spell `?? panic` and not `?`.
  - *Where `?` stays right*: absence with a sensible answer (a missing count is 0).  Where it
    is wrong: an image invariant, a table read by its own index — a silent default there turns
    a corrupt image into a plausible wrong answer.  `e!` is for exactly that.
  - *Spelling*: postfix `!`, parsed where `?` is (`operators.rs`, the postfix loop at
    `@PLN116`'s `x?`), so `v[i]!.f` chains as `x?.f` does.  The lexer's greedy `!=` keeps
    `x != y` a comparison (`x!=y` too, as today); a discharge is never an assignment target, so
    `e! = …` is no form.  Kotlin spells it `!!`, Swift `!`; one character matches `?`.
  - *Diagnostics*: on a statically non-null operand, the advice that `!x` on one already gets
    (DESIGN_DECISIONS_VALUES: *"`!x` on a statically non-null operand warns"*) — `e!` there is
    a no-op that says the author expected a null.
  - *Decision*: a new `@C` entry in DESIGN_DECISIONS_VALUES.md beside @PLN116, the owner's to
    ratify; the formal rule, LOFT.md, STDLIB.md's discharge table, the formatter and the LSP
    take it in one change, both backends and the C, with suite cells under the new rule.
  - *Effect on the ports*: 258 discharges become `!` (or an accessor's one `!`); each message
    the ports wrote by hand that says more than "is null" stays as `?? panic(…)`.
- **What `!` leaves** — the three shapes below still apply, now as structure rather than noise:
- **The box-of-one (176 sites): one accessor per schema field, in `ir.loft`** (with `!`, a
  one-liner each, or simply `inner[0]!` at the site).  `fn one(ns:
  vector<Node>) -> Node` (and `block_of`, `type_of` for the other boxes), the one place the
  image's invariant "a box holds exactly one" is discharged, with one message naming the box.
  The ports read `one(inner)` where they read `inner[0] ?? panic("a set has its value")`.
  **The representation, revisited with the owner (2026-10-30).**  The box exists because a
  recursive child needs an indirection: inline, `inner: Node` inside `Node` has no size, and
  the two indirections a program can write cost — `reference<Node>` a 12-byte address (the
  stride of every node grows, as an enum takes its largest variant's size), `vector<Node>` a
  4-byte handle read through `[0]`.  An owned single-child field as a new kind of EDGE was
  first declined (it would duplicate what a collection is); the owner's proposal — *"an
  abstract type that is only valid to use in combination with a given store; in almost any case
  you already know the store where the reference came from"* — is not that, and is designed as
  R-2s below.
- **The table reads (81): the accessors the ports already half-have.**  `binds::def`,
  `fvar`, `fn_of` exist; every port reads definitions, variables and types through them, so the
  discharge is written once per table.
- **`len(…) - 1` after a length test (9): an `(N-Domain)` elision the compiler may be missing.**
  A cell per shape (`if len(v) > 0 { v[len(v) - 1] }`, the same with the length in a local) on
  both backends: if the rule's guard elision does not reach it, that is a front-end gap against
  `(N-Domain)`, filed and fixed as one, and the discharges go when it is.

**R-2s — the out-of-line field (the owner's store-relative reference).**  An out-of-line struct
field: the same VALUE semantics as an inline struct field, a different layout.
- *Layout*: the 4-byte record number of a record in the HOST's store — `Parts::ChildRec`
  (P213), which already compares deeply (`database/equality.rs`), copies and frees by cascade
  (`database/allocation.rs`), round-trips the image (`ir_store.rs`, `ir_read.rs`) and exists in
  the C runtime (`LOFT_T_CHILD`, `loft_rt.h`, `loft_image.h`); today only closure records use it.
- *Read*: `n.inner` is the child with the host's store attached, `(store(n), rec, 8)` — a view,
  as every struct projection is (`@C86`).  The 4-byte form never leaves the field, so a variable
  always holds an ordinary reference: "valid only with its store" holds by construction.
- *Write*: `n.inner = x` copies `x` into the host's store, as an inline field's assignment
  copies; a fresh record already in that store with no other owner is moved.  A constructor
  (`NdSet { var: 3, inner: x }`) is the same write.
- *Copy, `==`, `{n}`, JSON, parse*: as an inline field — deep, nested, no `$ref`: a child, not a
  link (links across stores are `reference<T>`, loft#2268).
- *One routine per type (owner, 2026-10-30: "keep the type of its ancestry so we can reuse the
  display routine of the original object")*: `Parts::ChildRec(content)` keeps the type it holds,
  so every walker's arm is ONE indirection — the record number, in the host's store, at
  `(store(host), rec, 8)` — followed by `content`'s own routine, unchanged: display, JSON, parse,
  copy, release.  Equality already works so (`eq_walk` with `content`), and the C runtime's
  `LOFT_T_CHILD` arms copy and release through `d->elem` the same way; the formatter's arm,
  today the debug shape `ChildRec(rec)`, becomes the same follow-then-recurse.  In the language
  the field's type stays `Node` — code, diagnostics and the debugger show `Node`;
  `child_rec<Node>` is a layout row, never displayed.
- *Absence*: `inner: Node?` is record number 0, which `ChildRec` already reads as null.
- *When*: automatically, for a field whose type would contain its host by value — inline
  layout is impossible there, so the compiler knows; no new syntax.
- *Cost*: 4 bytes a child, the box's handle without the vector record and its length word; the
  image shrinks.
- *What it takes*: the rule in `formal/layout.md` and the write in the binding rules; a
  `formatting.md` arm (today's format arm is a debug shape, `ChildRec(rec)`); the parser's field
  read and write and the type check that admits a recursive field; `--native` codegen; the
  debugger and LSP display; a `@C` entry for the owner.  For the port: `ir.loft` declares
  `NdSet { var: integer, inner: Node }`, the Rust writer changes in lockstep (an image format
  bump), and the 176 box discharges go.

**R-2s as the program sees it.**  Nothing new to write: a field whose type contains its host
is laid out out-of-line by itself.

```loft
enum Expr {
  Num { v: integer },
  Add { l: Expr, r: Expr },
  Neg { e: Expr },
}

struct Tree {
  key: integer,
  left: Tree?,
  right: Tree?,
}
```

A struct whose recursion runs through a NON-null field has no finite value, so it is refused —
an enum's other variants end its recursion and need no such rule:

```
error: `Tree.left: Tree` makes every Tree contain another Tree — make it `Tree?`
```

Building and reading are ordinary constructors, matches and field reads:

```loft
fn eval(e: Expr) -> integer {
  match e {
    Num { v } => v,
    Add { l, r } => eval(l) + eval(r),
    Neg { e } => -eval(e),
  }
}

fn main() {
  e = Add { l: Num { v: 1 }, r: Neg { e: Num { v: 2 } } };
  println("{eval(e)}");                  // -1
  t = Tree { key: 5, left: Tree { key: 3 } };
  l = t.left;
  if l {
    println("{l.key}");                  // 3 — the guard narrows the local to `Tree`
  }
}
```

A write has an inline field's value semantics:

```loft
t.left = Tree { key: 4 };   // a fresh record: moved into t's store, nothing copied
t.right = t.left;           // a copy: the two subtrees are independent afterwards
t.left = null;              // the old subtree is released, all the way down
l = t.right;                // a view, as every struct projection is (@C86)
```

The second line deep-copies a subtree, as assigning an inline struct field copies; two places
SHARING one node is a link, and a link is `reference<Tree>`.  Display, comparison and JSON are
those of a nested inline value, through `Expr`'s own routines:

```loft
println("{e}");    // {l:Num {v:1},r:Neg {e:Num {v:2}}} — a top-level variant, as every one
println("{e:j}");  // {"Add":{"l":{"Num":{"v":1}},"r":{"Neg":{"e":{"Num":{"v":2}}}}}}
assert(e == Add { l: Num { v: 1 }, r: Neg { e: Num { v: 2 } } });   // deep, not by address
```

The port, before and after (`selfhost/src/ir.loft` and its readers):

```loft
// before
NdSet { var: integer, inner: vector<Node> },
NdSet { var, inner } => set(l, b, var, inner[0] ?? panic("a set has its value")),

// after
NdSet { var: integer, inner: Node },
NdSet { var, inner } => set(l, b, var, inner),
```

**R-2s, against language safety (owner, 2026-10-30: "evaluate this against language safety,
when will this model break, can we mitigate it?").**  The model is safe while three invariants
hold; each break below is one of them failing, with what already guards it, the mitigation,
and the cell that shows it.

- **I1 — a child lives in its host's store.**  Its 4-byte number is meaningful only there.
- **I2 — only its host ends a child** (a null write, or the host's own release), and nothing
  reads it afterwards.
- **I3 — no walk's depth is the data's length.**

| # | where it breaks | how bad | already guarded | mitigation | cell |
|---|---|---|---|---|---|
| S1 | a VIEW outlives a write: `l = t.left; t.left = null` (or a reassignment that frees the old child and mints a new one) leaves `l` naming a released record — I2 | silent use-after-free; the worst | `@C86` materialises a projection when its container is reshaped; nothing yet calls an out-of-line write a reshape | (a) a non-null write overwrites the EXISTING child in place — copy the value into a fresh record, release the old child's claims, then move the fresh bytes into the old record number — so a view stays valid and sees the new value, exactly as an inline field's view does; (b) a null write is a reshape: a live view rooted at the field is materialised (the scope pass, as for a container) or the write is refused, as a reshape under a live `&` is; (c) the released child is entombed (`@FR-H-Removed`), so any remaining reference reads null, not freed bytes | the S1 lines, values by hand, `LOFT_STRICT_STORES=1` and `LOFT_POISON=1`, all three backends |
| S2 | a long chain, `struct List { v: integer, next: List? }` of 100 000 nodes: copy (`allocation.rs:4531`), equality (`eq_walk`), format, parse, the claims validator and the C runtime's `loft_copy_claims` all recurse once per level — I3 | a stack overflow: a crash, loud | release is iterative (`remove_claims` collects `OwnedChild` in a worklist) | each walker iterates its LAST out-of-line field instead of recursing on it (a chain becomes a loop) and keeps an explicit stack for the rest; a step of its own before the default flips | a 1 000 000-node list built, copied, compared, formatted, parsed back and released, all three backends, under the 2 GiB ceiling |
| S3 | a copy path that moves a host's bytes without the claims walk would carry a record number into a store where it names another record — I1 | silent corruption | the child counts as heap (`derive_facts`, `allocation.rs:5571`: `ChildRec → (true, true)`), so every flat-copy fast path takes the claims walk; release refuses a child outside its host's store (`owned_edge_in_store`, `refuse_owned_edge`); the `[cr-check]` validator (`allocation.rs:3296`) | under `LOFT_STRICT_STORES=1`, the read (`ref_from_child_rec`) also checks the number is a live record of the host's store; the store exchange (`@FR-H-SwapIn`) already declines a type holding one (`tree_holds_no_stored_refs` answers no) | a host copied into another store, a vector of hosts grown and moved, a placed buffer, a `par` result returned — each then read and freed |
| S4 | an overlapping write: `t.left = t` or `t.left = t.left.left` — the value is inside what the write releases — I2 | silent use-after-free | the bytecode writer's own rule for stores (`@FR-O-Detach`: release after the value is read) | S1(a)'s order already covers it: the value is copied into a fresh record BEFORE the old child's claims go | both lines, values by hand, `LOFT_POISON=1` |
| S5 | an out-of-line field in a KEY (a hash, sorted or index element keyed on it): the search walk panics on a `ChildRec` (`database/search.rs:220`) | a crash | none | a key field may not be out-of-line: a compile-time refusal naming the field, as other unkeyable types are | the refusal |
| S6 | binary file I/O of a host (`f#write(t)`, `f#read`): `database/io.rs:308`, `:508` panic on a `ChildRec` | a crash | none | the arms written (the child's record follows its host in the file), or until then a compile-time refusal of binary I/O for such a type — never a run-time panic | write and read back, value equal |
| S7 | a store merged or upgraded: a `par` worker's result, an image read back under a changed schema | silent if a number is not remapped | a copy into the parent goes through the claims walk, which claims the child afresh; a whole store adopted keeps its numbers; `upgrade.rs` knows `ChildRec` | a cell per path, before the default flips | a `par` worker building a tree; an image of a type that gains an out-of-line field |
| S8 | a `reference<Tree>` (a LINK) to a child: the child released by a null write while the link lives | silent use-after-free | `@FR-H-Removed` reads a removed record as null (D6, closed) — if the child's release entombs it | S1(c) — the release entombs | the link read after the null write: null |

What the model never does: a child is never shared (every write copies, so no cycle can be built
from children; sharing is a link), and the 4-byte form never leaves its field (a variable always
holds a full reference).  The steps below take S1(a)/(b)/(c) in step 5, S3's strict check and S5/S6
in step 6, and S2 as a step of its own before the default flips.

**R-2s, the steps.**  Each step lands green on its own.  Until the last-but-one the feature is
behind an opt-in switch (`LOFT_OUT_OF_LINE=1`), so the default keeps refusing a recursive type
exactly as today, and every step's cells run with the switch on.

1. **The rules, no code.**  `formal/layout.md`: the out-of-line field — its layout (a record
   number in the host's store, `Parts::ChildRec(content)`), absence as record 0.
   `formal/types.md`: a recursive STRUCT field must be nullable; an enum variant may recur.
   `formal/binding.md`: the write — a copy into the host's store, a fresh same-store record
   moved, a null write releasing the old child; a read is a projection, a view (`@C86`).
   `formal/formatting.md`: rendered and parsed as the nested value.  A `@C` entry in
   `DESIGN_DECISIONS_VALUES.md`, for the owner.  The suite cells of the examples above are
   written under the new rule (with `@EXPECT_ERROR` until the feature lands).
2. **Accept the cycle (behind the switch).**  `data.has_value_cycle` (`src/data.rs:8019`) and
   its two refusals (`src/typedef.rs:572`, `src/parser/objects.rs:6121`): a cycle through a
   nullable struct field or an enum variant's field marks those fields out-of-line; a cycle
   through a non-null struct field gets the message above.  Cells: the refusal (both forms),
   and a recursive type that now type-checks.
3. **Lay it out.**  `src/typedef.rs`'s field layout gives an out-of-line field the type
   `database.child_rec(content)` — the closure record's precedent at `typedef.rs:1456–1480`.
   Cells: `LOFT_DUMP_TYPES` shows the 4-byte field; `layout_golden` unchanged without the
   switch.
4. **Read it.**  `src/parser/fields.rs`: a field read of an out-of-line field is
   `OpRefFromChildRec(OpGetField(host, pos))` (the closure read at `fields.rs:2770–2827`;
   `default/01_code.loft:1573`, `Stores::ref_from_child_rec`, `database/structures.rs:1614`),
   and so is a `match` arm's binding of a variant field.  Cells: `eval` above, on the
   interpreter.
5. **Write it.**  `src/parser/objects.rs`: a constructor builds the child in place with
   `OpChildRec(field, tp)` (`default/01_code.loft:1582`, `Stores::new_child_rec`,
   `structures.rs:1633`) and fills it as a literal.  An assignment of a value to a PRESENT child
   overwrites it in place (S1(a)): the value copied into a fresh record, the old child's claims
   released, the fresh bytes moved into the old record number; to an absent child it mints one.
   A null assignment releases the child, entombed (S1(c)), and is a reshape for the views rooted
   at the field (S1(b), the scope pass's view walk).  The move of a fresh same-store record is a
   later optimisation: copying first is always correct.  Cells: the write block above and S1,
   S4, S8, values and lengths by hand, `LOFT_STRICT_STORES=1`, `LOFT_POISON=1`, no leak.
6. **The walkers.**  Follow-then-recurse with `content`'s own routine: the two format arms
   (`database/format.rs:1731`, `:2497`, today the debug shape `ChildRec(rec)`), the text parse
   (`format.rs:441`, `record_text_parse`), binary read and write (`database/io.rs:308`, `:508`,
   which panic on a `ChildRec` today), the search walk (`database/search.rs:220`).  A closure
   record keeps its own shape (its content type is a closure record's).  Equality
   (`database/equality.rs:83`), copy and release (`allocation.rs`), the image
   (`ir_store.rs:664`, `ir_read.rs:1022`) and the descriptor already follow the child.  The
   strict read check (S3), the key refusal (S5).  Cells: format, JSON, parse back
   (`parse(format(e)) == e`), binary write and read back (S6), S3, S5, S7.
7. **The backends.**  `--native`: the generator already lowers the two operators for closure
   records (`src/generation/mod.rs`, `src/native.rs`); the field read and write are checked
   there.  The C: `child_record_operator` (`src/c/body/misc.rs:250`, `loft_ref_from_child_rec`,
   `loft_new_child_rec`) and the runtime's `LOFT_T_CHILD` arms (`src/c/loft_rt.h:2487`, `:3100`).
   Cells: every cell of steps 2–6 on all three backends.
8. **The tools.**  The debugger's value display and the LSP's hover show the field as its
   type (`Node`), never `child_rec<Node>`.
8b. **No walk deeper than the data's nesting (S2).**  Copy, equality, format, parse, the
   validator and the C runtime's copy iterate the last out-of-line field and keep an explicit
   stack for the rest.  Cell: the 1 000 000-node list, all three backends.
9. **On by default.**  The switch becomes the opt-out `LOFT_NO_OUT_OF_LINE`; `LOFT.md` gains the
   recursive types; the refusal's old cure (`use reference<T>`) is reworded to name both — a
   child (out-of-line, by value) or a link (`reference<T>`); X24 closes.
10. **The port.**  `selfhost/src/ir.loft` declares its children by value (`NdSet { var: integer,
    inner: Node }`), the Rust image writer and reader (`src/ir_store.rs`, `src/ir_read.rs`)
    write and read them as child records in lockstep, the image format's version bumps (an old
    image converted or refused by name), and the ports lose the 176 box discharges.  Receipts:
    `cargo test --release --test ir_schema_roundtrip`, and every comparison layer of
    `selfhost/compare/compare.py` unchanged on the kernels, the sample and the census.

**Verification.**  For R-2s: suite cells under the new rule — a recursive enum built, read,
written, copied (the copy independent), compared, formatted and parsed back, a nullable child
absent — on the interpreter, `--native` and the C; `ir_schema_roundtrip` for the image; the
comparison layers unchanged on the ports.  For `e!`: suite cells under `(N-Present)` — present (the value), absent
(the halt, its message naming the expression and its line), chained (`v[i]!.f`), a reference
and a bare enum (what `?` refuses), a statically non-null operand (the advice) — each on the
interpreter, `--native` and the C.  For the ports: the same receipt as R-1 (every comparison
layer unchanged); the count of `?? panic` in the three files is reported before and after.

## R-3 — a write into a copy: the bug the port met first — BUILT

**Built.**  The rule cell answered first: `(F-Ret)` (calls.md) makes a call that returns a
projection of its argument hand back a copy, so `v = fvar(…); v.x = …` writes the copy on both
backends, and the cure is the lint.  `report_lost_field_writes` (`use_analysis.rs`) judges a
struct local bound by a copy of a place — another named local, a call returning a projection
of a named argument, a value-struct copy — field by field: a field written after the copy bind
and read nowhere, with the copy never used whole, is a `lost-write`.  Order-insensitive for
reads, so an under-approximation; a struct holding a `(Col-Group)` is skipped, since a write
through one member is read through another.  Over the corpus (2 856 programs), 307 compiling
library files and the ports: two reports, both genuine (`518-value-struct-local` wrote a copy's
fields to show the source untouched; the cell now reads them back).  Guard
`tests/scripts/a-field-written-into-a-copy-and-never-read-is-a-lost-write.loft`, its exact set
in `tests/dead_code_lint.rs`.  The design below is the record.

**Measured.**  The first bind-layer difference was a lost write: `v = fvar(b, d, var)` (a call
returning a projection of its argument) and then `v.stack_allocated = true`, which reached the
copy and not the image — every later set of the variable read as a first one.  The
`lost-write` lint (`@C86`, loft#1840; DIAGNOSTICS.md) was quiet, because it judges the copy
WHOLE: other fields of `v` are read after the write, so the copy reads as used.

**Rule / decision.**  `@C86`: a whole-value bind copies; a struct-typed PROJECTION (`a = vv[0]`)
is a view.  First step: read what `(B-Copy)` / the return rules say for a call that RETURNS a
projection of its parameter (`fn f(b) -> R { b.xs[i] ?? panic(…) }`, then `v = f(b); v.x = 1`):
is the bind a copy (the write is lost) or a view (the write lands)?  The two backends and the C
are measured on that cell.  If the rule says view and the run copies, that is a deviation, fixed
at its home and filed if `main` has it (silent-wrong).  If the rule says copy, the lint is the
cure.

**Design (the rule saying copy).**  `lost-write` judges each FIELD of a copy, not the copy as a
whole: a field written in a local that holds a copy of a place (a projection, or a call's
returned projection) and never read afterwards — neither the field, nor the whole value passed
on, written back or returned — is reported, with the cure the lint already names (`&` for a
write-through, or read the copy back).  It gates: ignoring it loses a write (DIAGNOSTICS.md §
Diagnostic tiers).

**Verification.**  The cells: the port's own shape (reported), a field written and read back
(quiet), a copy written then passed whole to a call (quiet), a write through `&` (quiet), each
on both backends; the lint's false positives measured over `tests/scripts`, the suite and the
libraries before it gates, as every new warning is.

## Order

R-1 and R-2's accessors are refactors of the ports and can go together, after B-1 and E-3 (the
receipt is the comparison).  R-2's elision cells and R-3's rule cell come first in their
designs, because each can turn into a defect on `main` — then they follow the bug policy.  The
two language questions (an owned single-child field; the per-field lint) are the owner's.
