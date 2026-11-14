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
`loft_frame_site`), the `LOFT_PROVED` labels, the census and trace lines the switches print
(`LOFT_TRACE_*`), which are compared as a layer of their own.

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

The image carries per definition whether its body is under the 32-bit regime
(`Definition.width32` in `ir.loft`, `Data::body_width32`): without it a `#width 32` file's unit
came out 64-bit from the image.

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
