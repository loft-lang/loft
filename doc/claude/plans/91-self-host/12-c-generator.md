<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Strand 12 — the C generator in loft

The C code generator (`src/c/`, the decisions it reads in `src/opt/`) written in loft, in
`selfhost/`.  Parent: [README.md](README.md) § Sub-arcs, row 12.  This doc is the strand's
design and its step list; a step's own design is written here before its code.

## Goal

A loft program that reads a program image and writes the C unit **byte for byte** as the Rust
generator writes it for the same program — in every form the unit is written in, the debug
forms included.  Nothing less passes: a decision left out, a temporary numbered differently,
a comment spelled differently is a different unit.

**Why byte-identical and not "equivalent":** it is the only pass condition that needs no
judgment.  Two units that differ in one line need a reader to decide whether the line matters;
two equal files need `cmp`.  It also carries every optimisation across with no separate proof:
the 3× bar the Rust generator holds is held by the same text.

## The rules of the port

These are the interpreter port's rules (strand 1b), unchanged:

1. **Close to the Rust design.**  One loft module per Rust module, the same function names,
   the same order of decisions.  A reader who knows `src/c/body.rs` finds his way in
   `cgen_body.loft`.  Where loft has a clearer spelling (a `match` for an `if let` chain) it is
   used; the structure is not re-invented.
2. **Clean loft.**  The natural spelling, no work-arounds in the port's source.  Where clean
   loft cannot be written — a missing construct, a wrong answer, a leak, a slow natural form —
   **the loft side is fixed** (this repository's compiler, runtime or standard library), with
   its regression test, and the port then uses the natural form.
3. **Loft that is incorrect is fixed where it is met**: a test or a formal rule
   (`doc/claude/formal/`) says what is right; the rule wins over the code.
4. **Clear error messages.**  Every diagnostic the port's source draws is read as a user
   would: if it does not say what is wrong and what to write instead, the message is fixed.
   The generator's own refusals carry the Rust generator's text, since they are part of the
   compared output.
5. **Files under 2500 lines**, one subject each, from the start.
6. **Every analysis is compared by its debug data, not only by the text it leads to.**  A
   decision of `src/opt/` or of the writer has an internal state — the facts it derived, what
   it admitted, what it declined and why.  Both implementations print that state in one
   format, and the survey compares it as a layer of its own, per function.  Where the Rust
   side has no dump for a state the port needs to see, one is ADDED there (an env-gated
   `LOFT_TRACE_<analysis>` line or a census row), and the loft side prints the same.  A unit
   that differs then names the first analysis whose state differs, instead of leaving a
   reader to work back from a line of C.

## Input and oracle

- **Input:** the program image `loft --emit-image` writes — the `Data` and the type schema —
  read as `ir.loft` records (`load.loft`).  The image is the Rust front end's until strand 1a's
  replaces it, so the generator is checkable from its first line.
- **The image is enough:** `LOFT_EMIT_FROM_IMAGE=1 loft --emit-c` makes the RUST generator
  write the unit from the round-tripped image instead of from memory.  Where that unit differs
  from the in-memory one, the image lacks something, and the fix is in the image
  (`selfhost/src/ir.loft` first, then `make ir-schema-regen`), not in the port.
- **Oracle:** the Rust generator's unit for the same program and the same form.

### The forms of the unit

Each is a separate comparison; a program agrees in a form or it does not.

| form | Rust entry | command |
|---|---|---|
| program | `c::emit_program` | `loft --emit-c out.c p.loft` |
| tests | `c::emit_tests_of` | the unit `loft test --native-c` builds |
| source lines (debug) | `c::emit_program_with_source_lines`, `resolve_line_resets` | G-0 names the command |
| shipped | `c::ship_units`, `functions_whole` | `--native-c --ship --native-emit out.c` |
| library unit / bodies | `c::emit_unit`, `c::bodies::emit_bodies` | a library build; `--emit-c-bodies` |
| browser, ESP, wasm | `emit_browser_program`, `emit_esp_program`, `emit_wasm_program` | after the forms above |

The **debug data** the unit carries is compared in the forms that write it: the `#line`
directives and their resets, the frame sites (`loft_frame_push` with file and line,
`loft_frame_site`), the `LOFT_PROVED` labels.  `cgen_survey.py --form debug` compares the
debug unit; the plain form carries the frame sites.

### The layers beside the unit

What an analysis knew, compared per function (rule 6).  A layer exists once both sides
print it; `cgen_survey.py` gets a `--layer <name>` per row as it is built.

| layer | what it shows | Rust side today |
|---|---|---|
| `shapes` | the C shape of every variable of a function (`variable_shapes`), parameters first | `LOFT_TRACE_SHAPES` — **compared**: `cgen_survey.py --cgen --layer shapes` |
| `reach` | every definition a unit reaches from its roots (`opt::names::reachable_functions`), sorted | `LOFT_TRACE_REACH` — **compared**: `--layer reach` |
| `functions` | the functions the unit writes, in the order it writes them | `LOFT_TRACE_REACH` (its second line) — **compared**: `--layer functions` |
| `counters` | every counted loop of a function as `loop_shape::range_counters` reads it | `LOFT_TRACE_COUNTERS` — **compared**: `--layer counters` |
| `nn` | the non-sentinel facts a function's operators are written under (`opt::non_sentinel`) | `LOFT_TRACE_FACTS` (its `[nn]` line); the loft side is next |
| `ranges` | the range facts a body's operators were written under (`opt::range`) | `LOFT_TRACE_FACTS` (its `[ranges]` line); the loft side follows `nn` |
| `census` | each rewrite a function admitted, by rule (`make rewrite-census`) | `LOFT_REWRITE_CENSUS`, `LOFT_REWRITE_CENSUS_FN` |
| `declines` | each rewrite declined, with its reason (`make decline-census`) | the decline census |
| `inline` | each function's statement count and its inline verdict | `LOFT_TRACE_INLINE` |
| per analysis | its own trace | `LOFT_TRACE_VERSION`, `_COLD`, `_VALUEREC`, `_HOIST_DECLINE`, `_PUSH_FILL`, `_LOOP_BUFFER`, `_KEPT`, `_ARG_COPY`, `_BIND`, … (`grep -rho 'LOFT_TRACE_[A-Z_]*' src`) |

A trace line is machine-read here, so its format is a contract once a layer compares it: tab
separated, the function first, no address or hash order in it.

## Layout

`selfhost/src` is one flat module directory, so the generator is files beside the others:

| loft | Rust | subject |
|---|---|---|
| `cgen.loft` | `c/mod.rs` (entry, `Cx`) | the unit's context and entry points |
| `cgen_unit.loft` | `c/mod.rs` (`emit_unit_for`, `unit_defines`, tables) | the unit's assembly |
| `cgen_names.loft` | `c/mod.rs` (`c_name`, `native_c`, `plain_native`, `type_word`) | names and the native table |
| `cgen_types.loft` | `c/mod.rs` (typedefs, `CType`, shapes) | C types of loft types |
| `cgen_body*.loft` | `c/body.rs`, `c/body/*.rs` | a function's body; one file per Rust file, `body.rs` split by subject |
| `cgen_show.loft`, `cgen_parse.loft`, `cgen_eq.loft`, … | `c/show.rs`, `parse.rs`, `eq.rs`, … | one per Rust module |
| `cgen_tidy.loft`, `cgen_split.loft` | `c/tidy.rs`, `split.rs` | the passes over the written text |
| `opt_<pass>.loft` | `src/opt/<pass>.rs` | a decision, read by the writer |

The entry program is `selfhost/front/cgen.loft` (image in, unit out), tests are in
`selfhost/tests/`, fixtures in `selfhost/kernels/`.  Read before writing a walk of its own:
`ir.loft`, `load.loft`, `types.loft` (the facts the writer asks of `Stores`), `lower.loft` and
`own.loft` (the IR walk the runner uses), `show.loft`, `ptypes.loft`, `create.loft` (a generator
in loft).  **`ir.loft` and `load.loft` are shared with the runner:** an addition is announced to
the self-host stream before it is made.

The C runtime (`src/c/*.h`) is C and is not ported: the unit includes it as text, so the port
needs the same bytes — embedded as the Rust build embeds them (`RUNTIME`).

## Steps

Each step ends with a number: how many programs of the corpus agree in the form it adds.

- **G-0 — the oracle.**  `selfhost/bench/cgen_survey.py`: for every program of the census's
  corpus (`selfhost/cover/census.py`: tests/scripts and the language suite), the Rust unit and
  the loft unit, per form: `agree`, `differ` (the first differing line, both sides),
  `refused` (the loft generator names what it does not carry), `noimage`.  A ranking of the
  first construct not carried, as `run_image_survey.py` ranks operators.  Before any generator
  code: the count of programs whose unit the Rust generator writes equally from memory and
  from the image, per form — the size of the corpus the port can reach today.
- **G-2a — the empty program.**  `fn main() {}`: the unit's assembly — defines, the runtime
  text, the tables, `main` — byte-identical.  This step finds how the runtime text reaches the
  loft program and settles `Cx`.
- **G-2b — straight-line scalars.**  Integer, float, boolean and character locals, the
  operators, calls of user functions, `if`, loops: `body.rs`'s core walk, the temporaries'
  numbering, the null and overflow forms.
- **G-2c — texts**, then **records and vectors**, then **keyed collections**, each with the
  `body/` modules it needs, the simplest kernel first.
- **G-1 — the decisions**, interleaved: a pass of `src/opt/` is ported when the first corpus
  program whose unit it changes is next in the ranking.  Its admissions and refusals are
  compared against `make rewrite-census` / `make decline-census` and its `LOFT_TRACE_*` lines
  before its text is.
- **G-2d — the rest of `body/`**: transient, deferred, apart, version, par, generator, files,
  chain, cold.
- **G-2e — the passes over the text** (`tidy`, `split`) and the remaining forms of the unit.
- **G-3 — building**: the shim and link driver, which spawns `cc`.

The order inside G-2 follows the survey's ranking, not this list, once the ranking exists.

### The order of the next ports

Each is a layer first (rule 6), then text.  The non-sentinel facts need, in this order:
`opt/iteration.rs` (`bounded_iteration_index`), `opt/facts.rs` (`solve`, the least and
greatest fixpoint), then `opt/non_sentinel.rs` — whose per-program memos (`RETURNS`,
`PARAMS`) become a context record, and whose program-wide parameter proof is ON for the C
writer (`begin_c_generation`).  The range facts need `opt/char_walk.rs` and `opt/range.rs`
(its interval arithmetic in `i128` is written with the null an overflowing `integer`
operation answers).  With both layers equal, the writer's core walk (`body.rs` `write`:
declarations, temporaries, statements, calls, `if`) starts producing text.

## What decides a step is done

- The survey's count for the step's form did not drop and rose by what the step named.
- The port's own source draws no warning, and every diagnostic met on the way was read
  (rule 4).
- Every loft-side defect met was fixed with its test, or — when it blocks nothing and needs a
  design — filed as the bug-filing policy says.
- `selfhost/tests/cgen*.loft` holds the step's kernels as a guard in the selfhost run
  (`tests/selfhost.rs`).

## Measured

| step | number | command |
|---|---|---|
| G-0, image against memory | 1075 of 1075 units agree (1 program has no unit) | `python3 selfhost/bench/cgen_survey.py` |

| passes, `tidy.rs` | its 9 Rust unit tests in loft; `drop_unnamed` a fixpoint on 8 kernels' units | `loft test tests/cgen_tidy.loft` (in `selfhost/`, through `tests/selfhost.rs`) |
| G-2a, the empty program | its unit byte-identical (1 435 365 bytes), plain and debug form | `cgen_survey.py --cgen [--form debug]` |

| corpus, loft generator | 0 of 1075 units (every one refused: `a function body`) | `cgen_survey.py --cgen` |
| layer `shapes` | 13 263 function rows over 1075 programs, all equal | `cgen_survey.py --cgen --layer shapes` |
| layer `reach` | 1075 programs, all equal | `cgen_survey.py --cgen --layer reach` |
| layer `functions` | 1075 programs, all equal | `cgen_survey.py --cgen --layer functions` |
| layer `counters` | 13 263 function rows, all equal | `cgen_survey.py --cgen --layer counters` |

The image carries per definition whether its body is under the 32-bit regime
(`Definition.width32` in `ir.loft`, `Data::body_width32`): without it a `#width 32` file's unit
came out 64-bit from the image.

## What is built

| loft | Rust | state |
|---|---|---|
| `selfhost/src/cgen_tidy.loft` | `src/c/tidy.rs` | whole: `tidy`, `tidy_unit` (with `@FR-R-RefParam`), `drop_unnamed`; the census line `R-RefParam` is not counted yet |
| `selfhost/src/cgen.loft` | `src/c/mod.rs` | the runtime's composition, `c_string`, the unit's assembly for a program whose `main` is empty; any other program is refused by name |
| `selfhost/src/cgen_types.loft` | `src/c/mod.rs` (`CType`, `Cx::ctype`, `local_ctype`, `variable_shapes`) | whole, a shape held as its word; a build under `--width 32` is not asked (the image does not say it) |
| `selfhost/src/opt_names.loft` | `src/opt/names.rs` | `reachable_functions` with its walks (`collect_calls`, `collect_fn_ref_literals`, `collect_int_fn_refs`, `collect_tuple_fn_refs`), the store instances and a closure record's drop cascade; a unit's lazy drivers are not roots yet; `written_functions` (the unit's list: no operator, no native the runtime implements, the root first) |
| `selfhost/src/loop_shape.loft` | `src/loop_shape.rs` | `range_counters` with the stop guard and the break test; `plain_for_body` and `wrapped_counter` are not ported yet |
| `selfhost/src/cgen_natives.loft` | `native_c`, `host_native_c`, `json_native_c` in `src/c/mod.rs` | generated: `selfhost/bench/cgen_natives_gen.py` (`--check` says when it is out of date) |
| `selfhost/front/cgen.loft` | `loft --emit-c` | `cgen <image> <out.c> <src/c dir> [--debug]`, `cgen <image> <out> --layer shapes|reach|functions`; exit 3 with `not carried: <what>` |

The generator reads the runtime's headers from `src/c` (and `loft-ffi/include/loft_ffi.h`) at
run time, in the order `RUNTIME` embeds them.

## Loft defects the port met

Each was fixed where it was met, with its guard (rule 2 and 3 of the port).

| issue | what | guard |
|---|---|---|
| loft#2483 | `x = e ?? return null;` answered the empty text for a `text?` function on the interpreter | `tests/scripts/2483-…loft` |
| loft#2484 | `for c in f()` over a hash a call answers freed its store twice, both backends | `tests/scripts/2484-…loft` |
| loft#2485 | a bind after its block ended moved the ended binding's type (a false gating warning), and a per-loop binding never ended | `tests/callarg_nstore.rs` |

Not fixed, noted: a `hash<T[k]>` local cannot be formatted (`"{h}"`: "Cannot format type"),
where a record holding one can.

## Open questions, answered by the first probes

1. How many corpus programs does the Rust generator write equally from the image and from
   memory, per form?  (G-0's first number.)
2. Does the image carry what the DECISIONS read that is not IR — the census state, the
   switches' environment, the per-unit facts (`Cx::frames`, `Cx::lazy`, `Cx::threads`)?  Each
   is derived from the IR in Rust; the port derives it the same way.
3. The generator's speed: the time the loft generator takes for a unit is measured beside
   the Rust generator's from G-2a on, per program of the corpus.
4. Hash iteration order: wherever the Rust generator walks a `HashMap` or `HashSet`, its
   output is sorted first or the unit would not be reproducible; each such site is ported with
   its sort, and a site found unsorted is a defect of the Rust generator, fixed there.
