// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Debugging Strategy: introspection CLI

The `--introspect` CLI and the pass-1 refusal audit.  Part of the debugging guide: [DEBUG.md](DEBUG.md).

---

## Introspection CLI (`--introspect`)

**Is a refactor byte-identical?  `scripts/introspect_diff.sh <before-loft> <after-loft>
[--env VAR=VALUE]…`** runs `introspect` (IR + bytecode + generated Rust, and stderr) with both
compilers over every corpus file (`tests/scripts`, `tests/docs`, `examples`) and prints one
line per DIFFERING file plus `IDENTICAL n/m` or `DIFFERENT k of m`; exit 0 / 1 / 2 (usage).
A refactor that claims to change nothing is verified by this and by nothing weaker — the suite
asserts VALUES, so a compiler that emits differently and computes the same values passes it.
Read the verdict from the script's exit, not a pipeline's; an empty corpus is a usage error,
not a verdict (a copy run outside the tree once read `IDENTICAL 0/0`); both binaries are
absolutised (a relative `target/debug/loft` resolved inside the file's directory and read as
DIFFERENT on every file); and both PARSE (`LOFT_NO_CACHE=1`) — `introspect` now always does,
because a warm bundle carries no variable table and rendered every variable as `name(65535)`.
@PLN153 phase 2 is its first customer: `IDENTICAL 1268/1268` under the default and under
`LOFT_NO_NULLFLOW=1` for the null-flow fold.

**Where does a nullable get peeled?  `LOFT_TRACE_UNWRAP=1`** prints one line per `τ? ⤳ τ`
peel and per bare-`null` conversion inside `Parser::convert` — the types, whether the caller
admitted it as a TEST (`admit=`), the slot it is stored into (`what=`), and the caller's
`file:line` (`#[track_caller]`).  It is the census that located @FR-N-Store's one home
(@PLN153 phase 3: 6014 peels over the corpus, all through one arm), and the first thing to
run when a store warns with the generic wording *"into a slot"* — that names the lowering
that has not said which slot it is.

**Which `==` compares identity?  `LOFT_TRACE_EQ_IDENTITY=1`** prints one line per `==` / `!=`
the parser lowers to `OpEqRef` / `OpNeRef` — `file:line`, the kind (`struct`, `struct-enum`,
`collection`, …) and both operand types — and nothing for a test against the `null` literal.
Before `@C91`'s flips those were the sites whose answer content `==` changed; since them, a
struct, a struct-enum and a collection compare by content and the trace names what is left
(two records of different struct types).  `scripts/eq_census.sh` runs it over this tree,
every library at `main` and read-only snapshots of the consumer applications, and writes
`index/eq_census.txt` — committed as the pre-flip census (guard: `tests/store_content_eq.rs`).

`loft --introspect <file>` packages the dump primitives behind one
flag, dumping bytecode + generated Rust + slot tables + per-fn type
tables to stdout (or per-section files).  No env vars, no test
harness.  Use it when you want to inspect compile-time state without
running the program.

### Sections

| Flag | Output | When to use |
|------|--------|-------------|
| `--show-bytecode` | Bytecode disassembly per fn | Codegen bugs, "is the right opcode emitted?" |
| `--show-rust` | Generated Rust (`--native-emit` shape) | Native-codegen bugs, rustc errors |
| `--show-slots` | Stack-slot table per fn (name, type, scope, slot, live interval) | Slot conflicts, lifetime bugs |
| `--show-types` | Per-fn variable type + dep table | **Dep-tracking bugs** — see below |
| `--show-ownership` | Per-binding store ownership (@PLN103): `Owned` / `Borrowed(base=X)` (a live alias of X — the dangerous case) / `Owned (backing=…)` (owns via a delivery buffer) / `Join(base=X)` (a runtime owned-or-borrow split) / `Borrowed(caller-arg)` / `— (scalar)`, plus a per-return `delivery:` line (materialised / owned / borrows). **Also flags the loft#568 interpreter-orphan class STATICALLY** — `⚠ loft#568: owned text returned by value (…) — no &text retbuf` on a text return backed frame-locally with no delivery buffer (the interpreter orphans the `String`; native RAII drops it). Opt-in. | **Store-lifetime / owned-vs-borrowed bugs AND suspected text-return leaks** — reach for this before ASan: it names the leaker class without a sanitizer run. Verdict is backend-shared. |
| `--show-resolution` | Which names each source can SEE: one row per source with its `defined` and `visible` counts, then every import alias, and the `context:` line (stdlib dir + `--lib` paths). Opt-in. | **"Unknown function" / "Library not found" on a name that should resolve** — see below |
| `--why <name>` | The same section narrowed to one name: where it is defined, and every source it is reachable from. Implies `--show-resolution`. | The same, when you already know which name is missing |
| `--bc-roundtrip` | Re-assemble each fn's bytecode from its own dump and compare (`ok`/`DIFFERS`) | Verify the dump is a faithful, editable bytecode representation — see [Bytecode round-trip](#bytecode-round-trip---bc-roundtrip) |
| `--json` (INSP.J) | One machine-readable JSON object over the included sections — a string field per section (`bytecode`/`rust`/`slots`/`types`, plus `ownership` / `resolution` if requested), in canonical order | An editor / agent / the LSP that wants a section by key instead of splitting on `=== header ===` lines; takes precedence over `*-out` / `--diff` |

Combine the four dump flags freely; they emit in fixed order, and
no flags = all four.  `--bc-roundtrip` is **opt-in only** (a
verification check, not a dump — it never runs in the no-flags
default).  `--all-fns` includes the default/* stdlib.  `--fn
<name>` filters to one function.  `--json` renders whichever
sections are selected as one JSON object (parseable by loft's own
`json` reader) instead of the text dump.

### A record layout is a MEASURED fact

Byte offsets are what `OpGetField` / `OpNewRecord` carry, so a wrong one is a wrong answer —
and they cannot be inferred from the declaration. `--show-bytecode` prints them, which is
the only place to read them from:

```
GetField(v1: ref(reference), fld=4) -> ref(reference) type=vector<float> 78
NewRecord(data: ref(reference), parent_tp=81, fld=1) -> ref(reference)
```

`GetField`'s `fld` is a byte OFFSET; `NewRecord`'s is a field INDEX. Two things that look
alike and are not, and both appear as `fld=`.

Neither a hand-built `Stores` table in a unit test nor a reading of the field types will give
you the same numbers. `enum Shape { Circle { limbs: vector<float> }, Square { s: float } }`
puts `limbs` at 4 (a 4-byte collection handle after the discriminant) and `s` at 8 (float
alignment) — a unit test that assembles the same shape through `Stores::structure` /
`Stores::field` runs none of the layout pass and answers otherwise (loft#977). When a test
needs two fields to collide, `assert` the collision in the test so the premise cannot drift.

### `--show-resolution` when a name will not resolve

A name resolves only if the source you are calling it from can **see** it. loft
numbers sources: `0` is the standard library, `1` is your program, `2` and up are
the libraries it `use`s. A definition is visible in its own source; a `use` adds an
**alias**, so the name is also visible in the source that imported it.

When `Unknown function foo` or `Library 'bar' not found` appears for a name you
believe is there, this section shows which of those steps did not happen:

```sh
loft introspect prog.loft --show-resolution --lib lib/
```

```
context: stdlib="…/default"  lib_dirs=["…/lib"]
sources:
  0    defined 650    visible 650    std (…/default/01_code.loft)
  1    defined 1      visible 2      …/prog.loft
  2    defined 1      visible 1      geom (…/lib/geom.loft)
aliases (1 import binding):
  src 1    <- src 2    #650    n_hex_distance
```

Read it in three steps:

1. **`context:`** — the paths this run searched. `lib_dirs=[]` when you passed
   `--lib` means the flag never reached the session, so no library could load. That
   is a whole class of bug, visible without running the program.
2. **`defined` vs `visible`** — `defined` counts the source's own definitions,
   `visible` counts every name it can reach. Source 1 above defines 1 and sees 2:
   the extra one is the import.
3. **`aliases`** — one line per imported name. `src 1 <- src 2` reads *"source 1
   can see this because source 2 defines it"*. An **empty** list in a program that
   has a `use` means the import never took effect.

To ask about one name instead of reading the table:

```sh
loft introspect prog.loft --why hex_distance --lib lib/
```

```
`hex_distance` is #650, defined in source 2
  visible in source 1 (import alias)
  visible in source 2 (its own)
```

`is not defined in any source` means the library was never parsed — check
`context:` first. Listed as defined but **not** visible from source 1 means the
`use` is missing or did not apply.

### `--show-types` for dep-tracking bugs

The `--show-types` section renders each variable's full type via
`Type::show()`, including the dependency suffix (`text["a"]` =
text borrowed from `a`).  Designed to surface dep-propagation
bugs at a glance — exactly the shape that hid P197 (a `text`
element from a tuple struct field that should have carried the
host as a dep but didn't).

```
fn n_first -> text["a"]:
  #    arg  name                     type [deps]
  ----------------------------------------------------------------------
  0         a                        ref(A)
  1    arg  s                        &text
```

Compare the function's return-type deps against what you expect.
If a returned `text` should track a host but the table shows
plain `text` (no `[host]` suffix), the dep was lost in
`get_val::Type::Tuple`, `field()`'s `t.depending(*nr)`, or
`Type::depending`'s recursion.

#### `--trace` — per-expression tape

Add `--trace` to surface the type at *every* chaining step, not
just the final variable.  Critical for nested expressions where
one intermediate step might lose a dep:

```
$ loft --introspect --show-types --trace foo.loft

fn n_first -> text["a"]:
  #    arg  name                     type [deps]
  ----------------------------------------------------------------------
  0    arg  a                        ref(A)

  trace (per-expression types):
    4:7        ref(A)["a"]
    4:9        (text["a"], text["a"])  ← `.v` step
    5:2        text["a"]                ← `.0` step
```

The two-step tape makes the dep flow visible: `a` → `a.v` →
`a.v.0`, with each step carrying `["a"]`.  Before the P197 fix,
the `.v` step would have rendered `(text, text)` (no `["a"]`)
and the regression would have been obvious without reading any
code.

Implemented as a `Parser::trace_types` flag; `parse_part` calls
`record_type_trace(&t)` after each `.field`/`.tuple_idx`/`[idx]`/
`(args)` chaining step.  Position is the lexer's char-offset
within the line (so `5:2` means line 5, byte 2 of the source).

### `--diff <baseline>`: did my parser tweak change anything?

Capture once, edit, re-run with `--diff`.  Mirrors `diff -u`'s
exit codes (0 identical, 1 differs).

```bash
loft --introspect --show-bytecode myprog.loft > before.bc
# edit the parser
loft --introspect --show-bytecode --diff before.bc myprog.loft
```

Per-section `--*-out` redirects still write to their files;
`--diff` only covers stdout-bound sections.

### Labelled jump targets in the bytecode dump

`--show-bytecode` anchors every jumped-to offset with a `:POS<rel>`
label and rewrites each goto to reference it, so the dump reads as
editable labelled assembly instead of raw byte offsets:

```
 28[48]: GotoFalseWord(jump=:POS46, if_false: boolean)
 43[40]: GotoWord(jump=:POS58)
:POS46
 46[40]: ConstInt(val=2) -> integer var=r[16]:integer
...
127[40]: GotoWord(jump=:POS62)   ← backward loop edge, binds to the label
:POS130
```

Jumps bind to a label *identity*, not a byte offset, so inserting or
removing ops shifts no jumps.  The label `<rel>` is the target's
relative offset within the function (`collect_jump_targets` /
`instruction_len` in `src/compile.rs` — `instruction_len` decodes
each op's real length, so variable-length `ConstText`/`Iterate`
operands advance correctly).

### Bytecode round-trip (`--bc-roundtrip`)

`loft --introspect --bc-roundtrip <file>` dumps each function's
bytecode, re-assembles it from that text via
`compile::reassemble_function` (the inverse of the disassembler),
and compares to the original byte stream — reporting `ok` /
`DIFFERS` / `error` per function plus a tally.

```bash
loft --introspect --bc-roundtrip --all-fns myprog.loft
#   ok      n_classify  (139 bytes)
#   ok      n_main      (95 bytes)
#   ── 201 identical, 0 differing/error ──
```

A clean run proves the labelled dump is a **faithful, editable
representation of the bytecode** — every byte is recoverable from
the text.  Constants encode inline (`ConstText` carries its escaped
string); jumps resolve from `:POS` labels; call targets dump as the
function *name* (`fn=n_classify`, relocation-safe) and static calls
as the native name, both resolved back to offsets on re-assembly.

**Why it's a tool, not just a test** — it's the front half of an
"edit bytecode *outside the parser*" loop: dump a function, change
an op / a constant / a jump / drop in a free, re-assemble, and the
round-trip confirms it's well-formed.  For any **stack-neutral**
tweak that is a real way to ask "what does *this exact* bytecode
do?" without going through the parser.

**Limits** (honest boundaries of the edit workflow):
- *Stack-neutral edits* (swap an op, change a constant, redirect a
  jump, add a free) re-assemble correctly — slot positions and the
  `Return` discard are unchanged.
- *Stack-depth or local-set changes* do **not** round-trip a hand
  edit: var slots are stack-relative (`pos = stack − slot`) and
  `Return(…, discard=N)` is the frame size, both of which shift.
  Re-deriving them needs the slot/layout pass (`scopes.rs`), not a
  text edit.
- The last 20% — **splice-and-run** (append the re-assembled
  function to the code array, repoint `code_position` + caller `to`,
  execute) — is **not built**.  Relative gotos make a single
  function relocatable, so it's a small, self-contained add when the
  need is real.

Implementation: `compile::reassemble_function` + `escape_text` /
`unescape_text` (`src/compile.rs`); the `Roundtrip` section in
`src/introspect.rs`.

### Native-codegen source map

The `--show-rust` (and any `--native` compilation) emits
`// loft:<file>:<line>` comments above each function header and
each statement.  `rustc` errors on `/tmp/loft_native.rs:1450` map
back to a .loft line by reading the nearest preceding comment.

```rust
// loft:/tmp/myprog.loft:7
fn n_first(stores: &mut Stores, mut var_a: DbRef) -> Str {
  ...
  // loft:/tmp/myprog.loft:8
  return Str::new(...)
}
```

When rustc reports a borrow-check error or type mismatch, scroll
upward in the generated file from the error line to the nearest
`// loft:` comment — that's the source line under suspicion.

---

## Which refusals are reachable before types resolve (`LOFT_AUDIT_PASS1`)

Prints `[pass1-site] <file>:<line>` for every diagnostic emitted while the parser is on its
FIRST pass. The audience is this repo, not a loft author.

It exists for one recurring class: a refusal phrased as a type REQUIREMENT that fires on
pass 1 may be refusing an *unresolved* type as a *wrong* one, which makes declaration order
decide whether a program compiles — against LOFT_DECLARATIONS.md § File structure's "in any order". Five
sites have been found and fixed (`call_op`, `parse_match`'s `!valid_enum` exit, both
text-index bounds, a spatial slice's limit). The fourth pair retro-broke the published
`markdown` 0.2.0; the fifth was found by ENUMERATING refusals after a 29-probe behavioural
sweep came back clean, because a probe sweep can only test shapes someone thinks to write.

Use it as the confirming half of that enumeration:

```bash
for f in tests/scripts/*.loft; do
  LOFT_AUDIT_PASS1=1 loft --interpret "$f" 2>&1 | grep '^\[pass1-site\]'
done | sort -u
```

**A firing site is not a bug — expect most of the list to be correct.** The defect is not
"refuses on pass 1"; it is "refuses on pass 1 a type that is merely UNRESOLVED". A name
collision belongs on pass 1, and `s[true]` is rightly refused there too, because the
deferrals cover only `unknown`. Measured 2026-08-21 over the 811-script corpus, **34
distinct sites fire on pass 1 and none of them was a new defect** — so read 34 as a
candidate list, not as 34 bugs. Of the 134 refusals a context heuristic had called
already-gated, 5 appear in that set and all 5 survive review: two are the text-index bounds
refusing genuinely wrong types, two are name collisions, and one — a struct-literal field's
`convert` failure — was the only real candidate by shape and probed clean.

**The asymmetry is the design, not a caveat.** `Parser::first_pass` is mirrored into an
atomic beside every write to it, so a write this instrument misses makes it report FEWER
sites, never a phantom one. That is what makes a printed site safe to act on: it is
measured. Silence is the other half and is only inferred — it means no program in the run
reached that site on pass 1, which is indistinguishable from "never reached at all". Pair
any silent site with a probe that reaches its diagnostic on pass 2 before recording it as
gated; without that, a dead path and a gated one look identical.

**Where the reading tells fit.** A `diagnostic!` sitting outside an `if !self.first_pass`
a few lines below it is visible without running anything, and it is how `fields.rs:2202`
was confirmed. Treat that as a confirmation aid, not a discovery instrument: it reads as a
tell only once you know the class, and a site whose gate is two functions up looks
identical to a correct one. **Enumeration finds these; this instrument keeps them found.**

## Where the two passes type one expression differently (`LOFT_AUDIT_RETYPE`)

Prints `[audit_retype] <origin> <file> <fn>::<var>  <pass-1 type>  ->  <pass-2 type>` for every
retype on PASS 2 of a variable that pass 1 had already typed as a different SHAPE.  Pass 2 starts
from pass 1's variable table, so each line is an expression synthesised twice, differently —
`formal/types.md (T-Syn)` asks for one type.  The audience is this repo.

```bash
for f in tests/scripts/*.loft; do           # on a COPY of the corpus: a sweep writes caches
  LOFT_AUDIT_RETYPE=1 loft --check "$f" 2>&1 | grep '^\[audit_retype\]'
done | grep -v '/default/'
```

**Read it filtered.** Width and borrow-list refinements are already quiet (`is_equal`, and the
borrow lists stripped).  Still listed, and correct: compiler temporaries (`__ref_`, `__ncc_`,
`___tret`, … — a slot reused or lowered), a captured local boxed into a cell (`set_type` to a
`Reference`), `#663`'s element-width adoption, the nullable-struct synthesis between the passes,
and inference that COMPLETES on pass 2 (a generic resolved there, so a keyed read turns `τ?`).
Measured over the 1916 corpus files: 936 lines, about 25 on user-named variables after
those filters; the ones that were defects were the variant join pass 1 read through a
`Rewritten` marker (D-types-24), and — followed to the one place pass 1's TREE is replayed, a
parameter default — two call defects (calls.md D-call-25, D-call-26).
