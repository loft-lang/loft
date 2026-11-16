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
| `iterations` | the `#index` of every `for e in v` loop its own bound keeps from overflowing (`opt::iteration::bounded_iteration_index`) | `LOFT_TRACE_COUNTERS` (its second line) — **compared**: `--layer iterations` |
| `nn` | the non-sentinel facts a function's operators are written under (`opt::non_sentinel`) | `LOFT_TRACE_FACTS` (its `[nn]` line) — **compared**: `--layer nn` |
| `ranges` | the range facts a body's operators were written under (`opt::range`) | `LOFT_TRACE_FACTS` (its `[ranges]` line) — compared: `--layer ranges`; differs where an accumulator steps under a character walk, until the walks are handed over |
| `writes` | whether a call of a function may write a store (`opt::writes::call_writes_store`) | `LOFT_TRACE_WRITES` — **compared**: `--layer writes` |
| `texts` | the text facts a body is set up with: the borrowed locals, the literal views, the tested borrows, the slice borrows, the lazy splits, the split tables | `LOFT_TRACE_TEXTS` — **compared** field by field: `--layer texts` |
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

### Running units first, then the rules (owner, 2026-11-15)

The order from here: small programs RUN through the loft generator's C with correct results,
under the plain switches (the rules not carried yet, off on both sides — a correct program,
with fewer optimisations); then the same programs take the rules back one by one until their
units are the Rust generator's byte for byte under the default switches.  A body that equals
the Rust one proves nothing about a unit that is never compiled: assembly, the tables and the
names numbered by first use are only checked by building and running the program.

- **U-0 — the harness.**  `LOFT_CGEN=<generator>` makes `loft --native-c p.loft` compile the
  unit the loft generator writes from the program's image (`<generator> <image> <out.c>
  <src/c>`) instead of the Rust generator's — the same driver, flags, link and run.  A
  generator that refuses the program is a refusal there, never a fallback to the Rust unit.
  `cgen_survey.py --run` counts per program: `pass` (the program's own asserts pass and its
  output and exit are the interpreter's), `fail` (the first differing line), `refused`,
  `nobuild` (the C compiler's first error); beside it, whether the unit equals the Rust plain
  unit.  The target set is the programs with a `main` whose every body the writer already
  writes equal under the plain switches (517 of 1 077 on 2026-11-15), smallest first.
- **U-1 — the unit's assembly** (`emit_unit_for`) for the target set: the defines
  (`unit_defines`), the type tables, the typedefs, the prototypes and bodies in the Rust order,
  the framed wrappers, the names a unit numbers by first use (`loft_call<n>`, `loft_tup<n>`),
  the twins, the tables' binding and `main`; each piece ported as the next program needs it.
  Done when every program of the target set passes.
  Measured (2026-11-15, the skeleton — wrappers, prototypes in the Rust order, the
  hot-helper markers resolved, the common defines, the `__wr` twins; every table refused by
  name): **39 of the 517 programs pass, all 39 units byte-identical to the Rust ones**, 0 fail,
  0 do not build.  What the rest refuse, by programs: the copy table and the defaults
  (`loft_types` 148, `loft_dflt` 120, a record's copy table 52 — the 60 `copy_type` /
  `describe_type` registrations), the function values' dispatchers (59), the file library's
  define with its case table (48), the show code (25), the tuple structs (13).  `--run`
  compares a unit with the Rust one from the same site (`--native-c`), which can differ from
  `--emit-c`'s.
  With the type tables (`cgen_tables.loft`: the copy table, the key tables, the enum names, the
  default images, each registered where the Rust writer registers it, and compared as the
  `tables` layer, `LOFT_TRACE_TABLES`): **347 of the 517 pass, all 347 byte-identical**; the
  `tables` layer agrees on 346 of the 347 units written (the one left registers a value
  record's copy types, a rule under the plain switches).  Next, by programs: the function
  values' dispatchers (59), the file library's define and case table (48), the show code
  (41), the tuple structs (13).
  With the function values (`cgen_fnval.loft`: `dispatch_code`, `dispatch_arm`, `drop_code`;
  `Cx::fn_values` registered where the Rust writer's `fn_value` is): **395 of the 517 pass, all
  byte-identical**.  Next: the show code (50), the file library's define (48), the tuple
  structs (15).
  With the show code (`cgen_show.loft`: `show_type`, `show_code`, `fields_fn`, `show_body`,
  `enum_body`; the rendered types join the `tables` layer as `shows=`): **445 of the 517
  pass, all byte-identical**; the `tables` layer agrees on all 445.  Next: the file library's
  define and its case table (48), the tuple structs (15).
  With the Unicode case tables (`cgen_unicode.loft`: `unicode_case`, `case_code`; the file
  library's `LOFT_PATH_FOLD` and the `to_lowercase` / `to_uppercase` calls register them; the
  tables layer gains `unicode=`): **475 of the 517 pass, all byte-identical**; the `tables`
  layer agrees on all 476 units written.  Next: the constants (`loft_const`, 16), the tuple
  structs (15), the frames' define (4).
  With the constants (`const_code`: each constant's store built as the runner builds it,
  `consts::build_store`, read back through `crt::store_bytes`; the unit's defines decided after
  the bodies, a constant ruling out `LOFT_NO_LOCKS`): **491 of the 517 pass**, 475 of them
  byte-identical.  The 16 others differ in the constant's words alone: the C runtime's
  allocator lays the same store out otherwise than Rust's (`Stores::database`), so the words
  hold the same values at other places.  Open for phase O, a choice: the image carries each
  constant's store words, or the generator builds them with a port of the Rust allocator.
  With the tuple structs (`typedef struct { … } loft_tup<n>`, numbered by first use, a tuple's
  members before it as `tuple_id` numbers them): **506 of the 517 pass**, 490 byte-identical
  (the 16 constants' words aside).  Left: the frames' define (4), "a tuple expected, a Void
  found" (4), a locked store (2), reflection (1).
  With a call's tuple result carrying its shape (`E.tup`, met where the `--native-c` site keeps
  a call the `--emit-c` site inlined), the frames' define (`LOFT_FRAMES`, the `StackFrame`
  offsets) and the locked stores' (`LOFT_FOREIGN`; no `LOFT_NO_LOCKS` for `#lock` or a mapped
  file): **516 of the 517 pass**, 500 byte-identical.  Left: reflection (1, `reflect.rs`).
  `LOFT_CGEN_IMAGE=<path>` keeps the image the `--native-c` site hands the generator.
  Over the whole corpus: an over-budget body (`(R-CSplit)`) is written whole — what gcc and
  clang compile from the Rust unit too — its MSVC parts not carried, and the bodies layer still
  counts it refused; five more rules the writer does not carry join the plain switches
  (`LOFT_NO_CHAR_WALK`, `LOFT_NO_LAZY_SPLIT`, `LOFT_NO_SPLIT_TABLE`, `LOFT_NO_TESTED_BORROW`,
  `LOFT_NO_TEXT_WINDOW`), honoured on the loft side where the Rust writer reads them (the
  range analysis takes no walk under `LOFT_NO_CHAR_WALK`, as `FnWriter::char_walks` is empty
  there).  **800 of the 1 078 corpus programs pass, 613 byte-identical**; the plain bodies
  layer 12 381 equal, 0 different.  The one `fail` is the harness's: `19-files.loft` reads
  paths from the repository root, and the run starts it in its own directory.
  With `OpPlaceRecord` (`@FR-H-RootExtent`'s foreign flag from the host's root type) and
  `LOFT_NO_DESTINATION` among the plain switches (`@FR-R-Destination`'s `__d` twins are not
  carried; the bodies it shaped were hidden behind `OpPlaceRecord` until now): **835 of the
  1 078 corpus programs pass**, 636 byte-identical; plain bodies 12 465 equal, 0 different.
  The benches (`bench/*/bench.loft` and the portal libraries' `bench/bench.loft`, compared by
  each routine row's hash, `cgen_survey.py --run --bench`): **45 of 61 pass**.
  With `OpAbsInt` / `OpPowInt`, the out-of-line child records (`OpChildRecInPlace`,
  `OpMoveChildRec`, `OpDemoteRec`, `OpReleaseRecIfDistinct`, `OpStoreBufOnly`), the typed hash
  removal (`@FR-R-TypedKeyed`'s removal clause, `loft_hash_remove_i64`), a `main` that takes
  the program's arguments, and the null-sentinel detach of a deferred first bind (binds.loft,
  shared with the bytecode writer: `22_expr` freed a store twice without it): **846 of the
  1 078 corpus programs pass** (644 byte-identical), **47 of the 61 benches**.  A
  `tests/scripts` program now runs from the repository root, as the suite runs it.
- **U-2 — the tests' unit** (`emit_tests_of`), so the language suite's cells (`test_*`
  functions, no `main`) run through it too.
- **O — the rules back.**  Over the programs that pass, one `PLAIN_SWITCHES` entry at a time:
  its port, then the count of units equal to the Rust ones with that switch on, until the list
  is empty and the units are equal under the default switches.  A rule's port never lowers the
  `pass` count.

### The order of the next ports

Each is a layer first (rule 6), then text.  What `opt/range.rs` asks of its port, read off the
Rust before the first line is written:

- its expression half (`range`, `op_range`, `rem_range`, `elem_range`, `type_range`) is
  interval arithmetic in `i128`; in loft an operation on two `integer` bounds that does not fit
  answers null, which is the same verdict (`fits`);
- `call_range` keeps a memo per callee and argument ranges (`CALL_MEMO`, keyed by the
  definition's address) and a nesting count (`LONG_DEPTH`): a record of the program's `Ranges`
  context, keyed by definition number;
- `seed_accumulators` finds "the call just before this step" by the IDENTITY of a node
  (`std::ptr::eq`): the port names a statement by its block and position instead;
- three loops walk a `HashMap` (`shaped`, `sets`, `round`); each entry is judged alone except
  in `seed_while_counters`, where a counter's seed may read a range another one just entered —
  the port walks them by variable number, and a difference there is a defect of the Rust
  order, to settle in Rust;
- its inputs are the `nn` facts (ported), the function's character walks
  (`char_walk::walks`, the self-host stream's) and a twin's seeds (none for a plain function).

The range facts need `opt/char_walk.rs` and `opt/range.rs`
(its interval arithmetic in `i128` is written with the null an overflowing `integer`
operation answers).  With both layers equal, the writer's core walk (`body.rs` `write`:
declarations, temporaries, statements, calls, `if`) starts producing text.

### The rules still switched off in plain mode

`cgen_w::PLAIN_SWITCHES` names it (1).  Each one's reach — the bodies that change when the Rust writer runs it (`cgen_survey.py --plain --assume SWITCH --layer bodies`; corpus / benches (`--files`) / test-only programs (`--tests`), 2026-11-16; a rule that adds REFUSALS instead needs writer operators first):

| switch | corpus | benches | tests |
|---|---|---|---|
| `VALUE_RECORD` | 510 | 114 | 175 |

`VALUE_RECORD` also feeds `APART`'s instance 2: its plan leaves the value records' functions
alone (`value_fns` of `opt_apart_value::plan`), a set that is empty while the rule is off.

Back in the writer: `KEYED_IN_PLACE` (`opt_keyed_place.loft`, `cgen_keyed.loft`), `TEXT_WINDOW`
(`cgen_win::open_text_windows`), `APART_MINT` (`opt_hoist::apart_mint`), `VR_PTR` (`cgen_value.loft`; its
`ValueRecords::params` half comes with `VALUE_RECORD`), `HAND_OVER` (`opt_hand_over.loft`, `cgen_rotate.loft`,
the `__nk` twin), `GROWN_HEADER` (`opt_apart::grown_headers`, `kept_growth_param`, the `__kh` twin,
`cgen_hoist::refresh_grown`), `FNREF_BUFFER` and `FNREF_PARK` (`cgen_fnref.loft`, the `loft_call<n>b`
dispatchers), `DISTINCT_VERSION` (`cgen_loop::distinct_version`, the store facts' `assumed` pairs),
`APPEND_TWIN` and `APPEND_LOOP_WINDOW` (`opt_append_twin.loft`, `cgen_append.loft`: the `__ap` twin,
its window clause, the loop windows), `DESTINATION` (`opt_destination.loft`, `cgen_dest.loft`: the
`__d` twin and the caller's chain), `CHAR_WALK` (`cgen_loop::char_walk_loop`), `LAZY_SPLIT` and
`SPLIT_TABLE` (`cgen_lazy.loft`: the `loft_ls` iterator, the `loft_stab` table, its free at every
exit; a borrowed bind leaves their pieces to their readers), `TESTED_BORROW`
(`cgen_text::borrowed_assign`, the `loft_own<v>` text), `CALLEE_INPUTS` (`opt_inputs.loft`,
`cgen_inputs.loft`: the `__inv` twin's inputs, the loop verdict's callee candidates, the hot
helpers an `__inv` twin's loop reaches, the ranged twin's input seeds), `GUARD_BOUND`
(`opt_below_len.loft`, the marked index slots below), `VERSION` (`opt_version.loft`,
`cgen_version.loft`: the leaf plan, the `__vp` precondition, the loop prelude's `vg<k>` flags, the
`__vf` fast twin, the guarded loop's version clause), `DEFERRED_COPY` (`cgen_deferred.loft`, and
`opt_lease.loft` for `lease::Frame::read_after`'s per-path liveness), `APART` (`cgen_apart.loft` for
all three instances: 3 — `opt_apart_vector.loft`, the `loft_apvec` local; 4 —
`opt_apart_records.loft`, the per-field buffers, the `__apr` twin, the boundary copy and the
rotate from parts; 2 — `opt_apart_value.loft`, the `loft_ap_<T>` value, the `__apv` twin, the
construction runs, the `loft_ap_store` boundary, the typedefs, the hoists off in its bodies),
`TRANSIENT` (`cgen_transient.loft`, its unit state `opt_transient.loft` apart so `cgen::Cx` can
hold it without an import cycle: the site plan and `lives`, the `__tr<params>[r]` twins written
on first ask with their rollback, the `lt_<g>` structs, zeroers and seals, the operator
allow-list, the text lifts and the work clause's `wr_call`, the store-plan rewrites off in a
transient body).  A rewrite that writes COPIES of IR nodes (the destination
chain's call and arm statements, an append site's call, the rotate's call, the keyed in-place
literal) writes them under `no_memo`: the Rust writer keys an invariant's memo and a guarded
chain's admission by node address, which no copy carries.
No port needed: `REBOUND_MOVER` (the C writer's `verdict` hands `hoistable` no owned buffers, so
`rebinds_alias_free` declines every rebind on either side) and `LOOP_RECORD` (decided and applied in
the IR, `opt::loop_record`; the C writes what the IR says).  Unswitched rules the tests corpus showed
missing, ported on the way: `@FR-R-SliceArg`'s pooled carrier (`cgen_value::slice_carrier`).
A rule is tried before it leaves the list with `CGEN_ON=LOFT_NO_X selfhost/bench/cgen_live.sh --diff FILE`
(`LOFT_CGEN_ON` tells the generator).
A rule leaves the list when both sides agree on the whole corpus with it on.  Where a coarse
switch covers several clauses, its finer switches take its place first and each clause is
ported on its own.

What the loft writer still refuses beside the rules, by bodies of the corpus: a body the
writer splits (272), `OpCoroutineNext` (115) and `yield` (107) — the generators — a
character walk (95), `OpPlaceRecord` (86), a lazy split or a split table (65).  Whole-unit
assembly (the first-use numbering of `loft_call<n>` and `loft_tup<n>`, the framed wrapper,
the twins) follows the bodies.

The helpers of a port, beside the survey: `selfhost/bench/cgen_cycle.sh` compile-checks the
generator, builds it with `--ship` and runs the plain and the default `bodies` layer (it prints
`N of 1077 units agree`, so a generator that dies on some units cannot pass as "0 different");
`selfhost/bench/cgen_one.sh FILE FUNCTION` prints the unified diff of one body, the Rust
writer's against the loft writer's, under the plain switches.  For running units (U-0/U-1):
`cgen_run.sh [--gen EXE|live] FILE` runs one program through the loft unit the way
`cgen_survey.py --run --plain` does and diffs its output against the interpreter's;
`cgen_live.sh` is the generator run from the LIVE sources on the interpreter (~20 s a program,
no 6-minute build), usable wherever a generator is (`LOFT_CGEN`, `--generator`, `--gen live`),
and `cgen_live.sh --diff FILE` compares its unit with the Rust one;
`cgen_build_survey.sh [N] [--check]` snapshots the sources, builds them and surveys the corpus
(against the suite at `tests/loft-suite.pin`) and every bench (`cgen_survey.py --run --bench`
with no `--files` takes `bench/NN_*` and the library benches under `$LOFT_PERF_LIBS`);
`cgen_msvc_diff.py WANT GOT` diffs two units with the `_MSC_VER` split copy left out.  The run
counts are gated: `cgen_build_survey.sh --check` holds the corpus and the benches to
`cgen_survey.baseline`'s `run|plain|program|corpus` and `…|benches` rows (every program passes;
no drop in `pass` or in units equal to the Rust ones), a printed store report masked as
`scripts/c_sweep.py` masks it.  All keep
their output in `$LOFT_CGEN_OUT` (default `$TMPDIR/loft-cgen`); the shared setup is
`cgen_env.sh`.

### A rule keyed on node identity: the answer marked in the IR

Several Rust analyses answer a set of IR nodes BY ADDRESS and the writer asks by address:
`opt/below_len.rs` (`@FR-R-Base`'s guard clause, the index slots a passed test keeps below the
length), `@FR-R-Version`'s plan (the decided `if`s, the raw accesses, the plain operators),
`@FR-R-DeferredCopy`'s binds, part of `@FR-R-ValueLocal`.  A loft node handed around is a copy
(`&a == &b` finds no match — measured), and two equal nodes, one in the set and one outside it,
rule out asking by structure.  The loft writer carries the answer IN the IR instead: the analysis
writes the function's code again with each answered node wrapped in a marker span (`line` 0, a
reserved `pos`), once per function before its first body is written (`opt_below_len::mark`,
`Cx::guards_marked`).  Every position reader skips a span at line 0 and every `unspan` sees
through it, so nothing else changes; the writer asks the slot it holds (`opt_below_len::guarded`).
A copy a rewrite makes keeps the marker, which the Rust address test cannot match: the writer
asks only outside such copies (`no_memo`).  The Rust side is unchanged.
`@FR-R-Version` keys its plan by node too: its analysis runs on a NUMBERED copy (each call and
each `if` condition in an ID span, numbered in a pre-order the markers do not move), and a plan
that holds rewrites the leaf's and its callees' code with each planned node's ID turned into its
kind's marker (`DECIDED_T`/`DECIDED_F` on a condition, `RAW` or `PLAIN` around a call); a
caller's calls of a leaf are numbered as SITE spans before its first body is written.  The
writer reads a call's marker in `expr_value` (`W.call_kind`, `W.call_site`, saved around the
call); `stmt` hands a marked call statement to `dropped` with its span, and a line-0 span notes
no frame site of its own.
`@FR-R-DeferredCopy` keys its binds, its dead statements and the liveness target by statement:
a function with an owner witness (`__own_<x>`) has every statement position (block and loop
operators, `if` arms, `Insert` items) wrapped in a DC ID span before it is written
(`cgen_deferred::number_function`); `stmt` hands the ID of the span it unwraps to the statement
inside it (`W.dc_pending`) and keeps the enclosing IDs (`W.dc_ctx`), since everything inside a
top-level statement ahead of the first deferred bind is dead.

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
| layer `iterations` | 13 263 function rows, all equal | `cgen_survey.py --cgen --layer iterations` |
| layer `nn` | 6 044 function rows (the functions that ask a fact), all equal | `cgen_survey.py --cgen --layer nn` |
| layer `bodies` | the whole corpus (1076 units), default rules: of 13 537 function bodies the Rust writer wrote, the loft writer wrote 2 633 equal and none different (a function that holds a record or a vector is refused there) | `cgen_survey.py --generator <built cgen> --layer bodies` |
| layer `bodies`, plain | the whole corpus with both sides under `cgen_w::PLAIN_SWITCHES` (the rules not carried yet, off): of 13 346 bodies, 12 138 equal and none different, 1 325 refused (the names a unit numbers by first use — `loft_call<n>`, `loft_tup<n>` — renumbered per body) | `cgen_survey.py --plain --generator <built cgen> --layer bodies` |
| layer `writes` | 13 263 function rows over 1075 units, all equal | `cgen_survey.py --generator <built cgen> --layer writes` |
| layer `texts` | 13 263 function rows over 1075 units, all six fields equal | `cgen_survey.py --generator <built cgen> --layer texts` |
| layer `ranges` | tests/scripts: 444 of 444 programs equal (2 939 rows), the character walks read from `char_walk::walks` | `cgen_survey.py --generator <interpreted cgen, LOFT_NO_IR_VALUE_RECORD=1> --layer ranges --scripts-only` |

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
| `selfhost/src/opt_iteration.loft` | `src/opt/iteration.rs` | `iteration_head`, `bounded_iteration_index`; `element_binding` is not ported yet |
| `selfhost/src/opt_non_sentinel.loft` | `src/opt/non_sentinel.rs` | whole: `non_sentinel`, the callees' answers, the program-wide parameter proof (a greatest fixpoint over every call site, `src/fixpoint.rs` written out), the counter and iteration seeds, the escapes; the per-thread memos are one `Proof` record |
| `selfhost/src/opt_range.loft` | `src/opt/range.rs` | `range`, `op_range`, `rem_range`, `elem_range`, `call_range` with its memo (`Calls`), `range_vars` with the counter, accumulator (`movement`, `call_step`, `step_down_by_trips`), `while` and tuple-element seeds; `plain_form` and `blocker` (the writer's questions) are not ported yet; the character walks arrive as `Walk { scope, hoist_null }`, empty until `char_walk.loft` carries both |
| `selfhost/src/cgen_w.loft` | `Cx`, `FnWriter`, `E`, `K` and the leaf routines of `src/c/body.rs` and `src/c/mod.rs` (`held`, `initial`, `as_int` …) | what the writer's files share, declared before either is read |
| `selfhost/src/cgen_fn.loft` | `src/c/body.rs` (`FnWriter`), `function` and the whole form of `framed` in `src/c/mod.rs` | the FUNCTION WRITER for a function whose variables, parameters and result are scalars (integer, boolean, float, single, enum, character): assignments, `if`, blocks with their tail capture, `return`, `break`, `continue`, the operators of those shapes (plain where the range proof holds), calls of loft functions with their `__nn` and `__rg` twins, frame sites, temporaries by `arg`'s rule, `(R-Tabulate)`; every other shape is refused by name (text, a record, a function value, a tuple, a `&` variable, a call of the standard library) |
| `selfhost/src/cgen_text.loft` | the text operators, `format_long`, `text_reads`, `view_choice`, `text_tail_view`, `format_into_dest`, `borrowed_bind`, `held_texts`, `expand_epilogues`, `park_work_texts` of `src/c/body.rs` | text locals, parameters and results, literals, the text and format operators, printing, a borrowed text's view, the work-text park; refused by name: a text read out of a store, a `&text` parameter and `(R-BufferReturn)`, a tested borrow, a lazy split or a split table, a text push window, a cold exit, a body `(R-CSplit)` may split |
| `selfhost/src/cgen_ref.loft` | `store_operator`, `free_operator`, `database_operator`, `ref_operator`, `field_operator`, `narrow_operator`, the plain arms of `vector_operator`, `keep_room_operator` | field reads and writes, `OpDatabase`, the frees, element reads, the scalar pushes; under the plain rules only |
| `selfhost/src/cgen_bind.loft` | `set_stmt_arms` from `keyed-null` on: `join_stmt`, `call_copy`, `element_copy`, `reading_view_copy`, `whole_value_bind`, `displacing_rebind`, `owned_vector_null`, `swap_safe` | a record or vector local's bind, on `binds.loft`, `own.loft` and `bgen_set::re_bind_of`; refused by name: keyed-null, a record from a call through a function value or a tuple member, a rebind of a `&` record |
| `selfhost/src/cgen_cold.loft` | `src/c/body/cold.rs`: `cold_plan`, `cold_arm`, `cold_if`, `cold_finish` | `@FR-R-Cold`'s loft clause under the plain rules — the helper's call in the body, the helper's own body written to learn whether it is refused (its text joins the unit with unit assembly); an arm is identified by equality, where the Rust writer keys it by address |
| `selfhost/src/cgen_split.loft` | `strip`, `idents`, `call_sites` of `src/c/split.rs` | `(R-CSplit)`'s size test only: a body over the budget of runtime-call sites is refused by name; the split's own text is not carried |
| `selfhost/src/opt_push.loft` | `opt::writes::fused_push`, `vector_path`, `push_operands`; `src/empty_slot.rs`; `opt::push_window::simple_invariant`; `hoist::hash_fill_loop` | the scalar push a rule may fuse, whether a vector is known empty, a loop-invariant expression, the hash a counted loop fills |
| `selfhost/src/opt_view.loft` | `hoist::view_extent_verdict`, `record_view_ptr`, `param_view_ptr`, `scalar_field_uses`, `free_before_use`; `body_writes::view_field` | `@FR-R-RecPtr`'s verdict without the store-apartness facts |
| `selfhost/src/cgen_recptr.loft` | `bind_param_ptrs`, `bind_record_ptr`, `rec_ptr_access` of `src/c/body.rs` | a record view's address taken once and its scalar field accesses through it; not the held-element address nor the text clause |
| `selfhost/src/opt_hoist.loft` | the base clause of `hoist::hoistable`, `header::vector_candidates`, `body_writes::rebound_vars` | the vectors a loop that writes no store may hold for its run |
| `selfhost/src/cgen_hoist.loft` | `hold_vectors`, `held_access`, `held_proof`, `place_of`, `index_arg`, `held_len` of `src/c/body.rs` | `@FR-R-Header`'s base clause: the held header, the reads, the length and the scalar element writes through it; the tiers above it are plain switches |
| `selfhost/src/cgen_loop.loft` | `loop_stmt` … `loop_core`, `range_guard` in `src/c/body.rs`; `body/peel.rs`, `body/sticky.rs`, `body/spec_chain.rs`, `body/chain.rs`; `opt::hoist::invariant_chains` | a loop over scalars with `(R-Peel)`, `(R-StickySum)`, `(R-SpecChain)`, `(R-GuardedChain)`, `(R-Invariant)` and `(R-RangedCall)`'s guarded copy; a node an analysis admitted is found by what decided the admission, not by an address; the holds of a vector, a record scalar, a push or a file are not asked |
| `selfhost/src/cgen_names.loft` | `c_name`, `twin_name`, `runtime_name`, `is_operator` in `src/c/mod.rs`; `duplicate_fn_names`, `disambiguated_fn_ident` in `src/opt/names.rs` | whole; a twin is a record of its flags, not a bit mask |
| `selfhost/src/opt_ranged_call.loft` | `src/opt/ranged_call.rs` | the gain test, `args_ranged`, `param_seeds`, and `guard_plan` for its variable clause; a hoisted record field is not a guard's leaf yet |
| `selfhost/src/opt_frame_calls.loft` | `src/opt/frame_calls.rs` | whole (`calls_a_frame`, `GuardFree`) |
| `selfhost/src/opt_tabulate.loft` | `src/opt/tabulate.rs` | whole |
| `selfhost/src/opt_writes.loft` | `may_write_store`, `call_writes_store`, `in_place_only_writer` and their allowances in `src/opt/writes.rs`; the memo and cycle guards of `src/opt/facts.rs` | the question with no tier on and no owned buffers; the tiers a loop's verdict turns on are not asked yet |
| `selfhost/src/opt_lazy_split.loft`, `opt_split_table.loft` | `src/opt/lazy_split.rs`, `opt::hoist::split_tables`, `split_call` and `one_op_wrapper` of `src/ir_shape.rs` | whole |
| `selfhost/src/opt_text_borrow.loft`, `opt_store_text_borrow.loft` | `src/opt/text_borrow.rs` (`borrowed_text_walks`, `text_escapes_with`), `opt::hoist::borrowed_discharge_temps`, `src/opt/store_text_borrow.rs` | whole for what the C writer asks (`local_roots`); the walk's re-read (`TextBorrow.read`) is not kept |
| `selfhost/src/cgen_texts.loft` | `borrowed_texts`, `literal_views`, `view_bind`, `work_view_bind` in `src/c/body.rs` | whole; the writer does not write a text yet |
| `selfhost/src/cgen_natives.loft` | `native_c`, `host_native_c`, `json_native_c` in `src/c/mod.rs` | generated: `selfhost/bench/cgen_natives_gen.py` (`--check` says when it is out of date) |
| `selfhost/src/opt_keyed_place.loft`, `cgen_keyed.loft` | `src/opt/keyed_place.rs`; `keyed_in_place`, `stage_literal_writes` of `src/c/body/keyed.rs` | whole: `@FR-R-InPlaceLiteral`'s keyed clause; a rewrite's C operand is an `NdRawExpr` the writer names with `raw_expr` (`FnWriter::raw`) |
| `selfhost/src/cgen_value.loft` | `Cx::vr_ptr_params`, `vr_param_read_only`, `tuple_arg_address` | `@FR-R-ValueRecord`'s pointer clause for the IR's tuples; the value records' half waits for `VALUE_RECORD` |
| `selfhost/src/opt_hand_over.loft`, `cgen_rotate.loft` | `src/lower/hand_over.rs`, `Stores::plain_vectors_of`; `rotate_site`, `nokeep` of `src/c/body.rs` | `@FR-R-HandOver`'s rotate clause and its `__nk` twin; an apart source rotates in from its parts (`cgen_apart::rotate`) |
| `selfhost/src/cgen_fnref.loft` | `src/c/body/loop_record.rs`; the kept-buffer half of `opaque_callref_stmt`, `call_ref` and `src/c/fnval.rs` | `@FR-R-FnRefBuffer` and its frame clause (the park slots) |
| `selfhost/src/opt_append_twin.loft`, `cgen_append.loft` | `src/opt/append_twin.rs`; `append_site`, `append_call`, `append_twin`, `append_twin_buffer_op`, `apw_operator`, `open_loop_windows` of `src/c/body.rs`; `Cx::ap_window` | `@FR-R-AppendTwin` with its window and loop clauses |
| `selfhost/src/opt_destination.loft`, `cgen_dest.loft` | `src/lower/destination.rs`; `destination_site`, `destination_level`, `destination_names`, `dest_scalar_set`, `dest_return` of `src/c/body.rs` | `@FR-R-Destination` |
| `selfhost/src/cgen_lazy.loft` | `lazy_split_bind`, `lazy_split_op`, `split_tab_bind`, `split_tab_op` of `src/c/body.rs`; the `loft_stab` part of `FnWriter::new` and `epilogue` | `@FR-R-LazySplit`, `@FR-R-SplitTable` |
| `selfhost/src/opt_inputs.loft`, `cgen_inputs.loft` | `callee_inputs`, `path_scalar`, `substitute_path`, `input_header_at` of `src/opt/hoist.rs`; `inv_inputs`, `twin_scalar_shape`, `Cx::hot_helpers` of `src/c/mod.rs`; `twin_inputs` of `src/c/body.rs`; `twin_seeds`, `scalar_inputs_ranged` of `src/opt/ranged_call.rs` | `@FR-R-Inputs` |
| `selfhost/src/opt_below_len.loft` | `src/opt/below_len.rs`, `forward_walk::check_body`; `guard_index` of `src/c/body.rs` | `@FR-R-Base`'s guard clause |
| `selfhost/src/opt_version.loft`, `cgen_version.loft` | `src/opt/version.rs`; `src/c/body/version.rs`; `version_fast`, the `version_raw`/`version_decided` sites and the call's fast form in `src/c/body.rs` | `@FR-R-Version` |
| `selfhost/src/cgen_deferred.loft`, `opt_lease.loft` | `src/c/body/deferred.rs`; `Frame::read_after`, `read_itself_after`, `readers_of` and `Pass` of `src/lease.rs` | `@FR-R-DeferredCopy` |
| `selfhost/src/cgen_apart.loft`, `opt_apart_vector.loft`, `opt_apart_records.loft`, `opt_apart_value.loft` | `src/c/body/apart.rs`, `src/c/body/apart_value.rs`; `src/opt/apart_vector.rs`, `src/opt/apart_records.rs`, `src/lower/apart_value.rs` | `@FR-R-Apart`, instances 2, 3 and 4 |
| `selfhost/front/cgen.loft` | `loft --emit-c` | `cgen <image> <out.c> <src/c dir> [--debug]`, `cgen <image> <out> --layer shapes|reach|functions`; exit 3 with `not carried: <what>` |

The generator reads the runtime's headers from `src/c` (and `loft-ffi/include/loft_ffi.h`) at
run time, in the order `RUNTIME` embeds them.

## Loft defects the port met

Each was fixed where it was met, with its guard (rule 2 and 3 of the port).

| issue | what | guard |
|---|---|---|
| loft#2483 | `x = e ?? return null;` answered the empty text for a `text?` function on the interpreter | `tests/scripts/2483-…loft` |
| loft#2484 | `for c in f()` over a hash a call answers freed its store twice, both backends | `tests/scripts/2484-…loft` |
| loft#2486 | an early `return` of a vector a `match` arm bound handed the caller a view into a freed store (an empty vector on the interpreter, a store freed twice on both) | `tests/scripts/2486-…loft` |
| loft#2490 | an early `return R { … }` from a function answering a small record `R?`, after a condition that calls another such function, answered null on the interpreter: the IR value-record pass left a literal that ends in its own `return` in its store form — fixed in `Exits::walk` (`src/value_record.rs`) | `tests/scripts/2490-an-early-return-of-a-small-record-literal-is-its-tuple.loft` |
| loft#2491 | the shipped C build refused `match f(v) { B { n } => vr(rv, n), … }`: the value-record site gate took a block's last operator for its value, and the match block frees its subject after the arms — fixed in `site_walk` (`src/opt/value_records/sites.rs`) | `tests/c_split/ship_value_before_frees.loft` |
| loft#2487 | a lambda written in a loop that captures a vector frees the vector's store at the end of the first round, both backends — **open**: filed with its cause and a verified workaround (bind the lambda before the loop); the port gives each walk a function of its own | none yet |
| loft#2493 | a `match` whose result an earlier arm made optional refused a later arm that is a block ending in a local of the base type | `tests/scripts/2493-…loft` |
| loft#2496 | a lambda capturing a local whose type pass 1 cannot know beside a `vector<boolean>` had no storage for the capture | `tests/scripts/2496-…loft` |
| loft#2497 | a walk `for x in f()` over a call's vector of a record declared later is refused, or — the record in a module that uses this one back — leaks the vector — **open**: filed with both repros; the writer's shared records are declared in `cgen_w.loft`, before the files that walk them | none yet |
| loft#2499 | a `match` with a leaving arm, last in an `if` branch that has an `else`, was dropped: the arm that matched did not run (silent) | `tests/scripts/2499-…loft` |
| loft#2498 | the C backend's transient form of a record registered itself as its own member | `tests/scripts/2498-…loft` |
| loft#2519 | a tail `match` over a call's result answered null from every arm once an arm built a vector a lambda captures (silent on the interpreter, refused by the C backend): the captured vector's guarded release (`free_unless_record_built`) was read as the match block's value — fixed in `is_scope_exit_cleanup` (`src/scopes/insert_free.rs`) | `tests/scripts/2519-a-tail-match-over-a-call-answers-its-arm-past-a-captured-release.loft` |
| loft#2485 | a bind after its block ended moved the ended binding's type (a false gating warning), and a per-loop binding never ended | `tests/callarg_nstore.rs` |
| (no issue) | a text function whose tail `match` reads a type declared below, called from above, aborted the compiler on the H5 two-pass contract: pass 2 promoted a work text to a hidden parameter pass 1 never published — fixed in `text_return` (pass 2 follows pass 1 for compiler temps too; 97b9527f3), and `deliver_body` answers its `match` directly again | loft-suite `calls/F-Ret/a-text-match-over-types-declared-below-returns-through-every-caller.loft` |
| (no issue) | `x = f(…) ?? return null` inside `f` itself (a recursive discharge) left `x` a view and leaked the copy once per call: the call-owns verdict asked `is_loft_defined`, false for the function being parsed — fixed in `null_coalesce_exit` (e0ec9e9e6) | loft-suite `heap/H-Free/a-discharged-call-answer-is-owned-by-its-binding.loft` |

Not fixed, noted: a `hash<T[k]>` local cannot be formatted (`"{h}"`: "Cannot format type"),
where a record holding one can.  A recursion over a tree that calls itself in tail position
draws `warning[tail-call-not-jumped]` (@C155, by design: such a call keeps its frame); the
port's recursive proofs go through a wrapper that binds the answer.

## Peers

The other streams this strand shares code and merges with, by the name their session
answers to: **loft3-ac** gathers the work branches into the pull requests and takes the sha
of each push of `laptop-c-perf`; **loft2-c0** writes the interpreter in loft
(`tuxedo-pln204-int32`) and owns `ir.loft`, `load.loft`, `binds.loft`, `own.loft` and
`char_walk.loft`.  This strand's session is **loft-c2**.  Where a session cannot be reached,
the note for it goes in the commit message it will merge.

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

## Review 2026-11-15 — the strand against the port method, and the interpreter stream beside it

Read on `laptop-c-perf` at its 81st strand-12 commit, two days after the design landed, with
the interpreter stream (`tuxedo-pln204-int32`, strand 1b) measured against the same items.

**The position.**  25,000 lines of loft in 42 files against the 66,000 of `src/c` and
`src/opt`; every analysis layer equal over the corpus (shapes, reach, functions, counters,
iterations, nn, writes, texts on every function row; ranges on 444 of 444 scripts); under the
plain switches about 11,300 to 12,100 of 13,300 bodies equal and none different; with every
rule on, 2,633; whole units byte-identical, the empty program only.  Twelve language defects
met, ten fixed with a guard.  Six of the 81 commits touched the Rust reference, each adding a
trace and none changing what it writes.  What the execution got right: the layers before the
text, so a differing body names the first analysis whose state differs; equal or refused by
name, never different, so the count reads as progress; and the reference kept as the truth,
with the one design question (a rule keyed on node identity) raised rather than settled
locally.

**What the strand owes, in order.**

1. **The position is committed and gated.**  Every number above was measured by hand and
   typed into this plan; no committed file and no CI step would notice a push under which
   five hundred bodies stop agreeing.  `cgen_survey.py` writes its per-layer counts and the
   plain-switch list to a baseline beside it, compares the next run against it, and fails on
   any drop or any `differ`; the run is a nightly's size (1,075 units), so it is a nightly
   job, and the selfhost cargo test keeps the kernels.  This is the port method's rule for
   every port, and the interpreter stream owes the same (below).
2. **No workaround for an open defect in the port's source.**  loft#2487 and loft#2497 are
   open and the generator's shape carries a detour for each (a function per walk; the shared
   records declared before the files that walk them), against rule 2 and the dogfood rule's
   "not even a temporary one".  Both are lifetime defects of the kind @PLN211 is clearing;
   they are fixed now and the detours taken out, or the port waits on them.
3. **Unit assembly before the next switch.**  Bodies are the bulk of the work and not the
   goal: the first-use numbering of `loft_call<n>` and `loft_tup<n>`, the framed wrapper, the
   twins and `split` are what turn equal bodies into a unit `cmp` accepts.  A whole unit also
   lets `scripts/c_sweep.py` compile the loft-written C and run it against the interpreter — a
   second oracle at no cost — and is where the 3× bar is actually carried across.
4. **The generator's own speed, per unit, beside the Rust generator's** — open question 3,
   promised from G-2a, has no number.  The pivot was made for compile time, and a survey runs
   that cost over 1,075 units each cycle.
5. **The helpers are committed.**  `cycle.sh` and `one.sh` live in a session's scratch
   directory, to be rebuilt from a paragraph; they belong in `selfhost/bench` beside the
   survey (BUS_FACTOR.md: everything needed is in the repository).

**The interpreter stream against the same five** (`tuxedo-pln204-int32`, 477 commits since
2026-11-12, 71,000 lines of loft, the C runtime shim 3,200 lines of C):

| item | strand 12 | strand 1b |
|---|---|---|
| position committed and gated | no | no — `run_image_survey.py` reads 1,193 of 1,194 and `compare.py` runs strict per layer, but neither carries an expected count, and `tests/selfhost.rs` keeps one emitted-image program |
| workaround for an open defect in the source | two | none found (`selfhost/src` carries the word nowhere); X23 is a register row, not a detour |
| the whole-artefact gate | not started | reached: the runner on loft-written code 1,135 of 1,194 agree, 0 differ; the rest of the layers measured per program (1b-compare.md) |
| speed measured against the reference | no | yes, continuously: `bench_speed.py` geometric mean 0.92 of `loft --interpret` over 94 routines, worst 2.25, from 2.03 two days before |
| helpers committed | `cycle.sh`, `one.sh` missing | `bench_speed.py`, `run_image_survey.py`, `fill_speed.sh` committed |

So the interpreter stream already does what items 2 to 5 ask, and the strand takes its
practice: the survey as the first thing a session runs, the speed number beside every
position, the scripts in the tree.  What neither stream has is item 1, and it is the one that
matters most as the commit rate rises: at forty commits a day a count that is only re-typed
by hand is a streak, not a gate.

On starting this strand before the interpreter is complete, which this epic's README ruled out
until 2026-11-14: it costs nothing, since the oracle is the Rust image and not the loft
interpreter; the one coupling is the shared IR files, and the one addition made there
(`Definition.width32`) is the kind the layout rule says to announce first.
