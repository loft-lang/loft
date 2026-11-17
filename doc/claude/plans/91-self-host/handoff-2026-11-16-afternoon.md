<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
<!-- size-exempt: a dated handoff record, read once (DOC_QUALITY § Maintainer docs 2) -->
# Handoff 2026-11-16 (afternoon, evening): gate E at 890 of 2172, a fast loop for the long tail

Branch `tuxedo-pln204-int32`, pushed; PR #2518 (loft3's gather) carries it up to 77024883f
(819 of 2151) and re-wrote gate E's baseline at ITS suite pin 898bc05 — after it merges, rebase
and re-write the baseline at the pin main then has.  Goal (owner, /goal): the full loft
interpreter, the in-between data of the two implementations compared at every layer.

## Where it stands (suite pin 508580956)

| Layer | Gate | Position |
|---|---|---|
| front end (gate E) | `selfhost/front/elab_gate.py` | **851 of 2158 agree, 1307 refused, 0 differ** (818 of the 2125 outside the cells; 765 at the day's start); baseline committed per code commit |
| writer / runner | `bgen_survey.py`, `run_image_survey.py` | as `handoff-2026-11-16-day.md`; their re-pinned baselines are still uncommitted in `../loft2-repin` |

The corpus is 2143 because the gate has a part of its own now: `selfhost/front/cells/*.loft`
(33 cells, all agreeing) — one shape per cell that the corpus does not hold ALONE.

## What gate E's remaining 1342 are

A long tail, not a few families: 301 programs are blocked by ONE refusal each, and those
are almost all different shapes (`--plan` admits five or six programs per family at best).
The measured rate with the loop below is about ten programs an hour.

**A lifted refusal uncovers differences.**  A program refused for one construct was never
compared in its other functions.  Of the eleven refusals lifted this session, most uncovered
one or more constructs the port had answered WRONGLY without refusing — a line marker, a
borrow list, a block type, a row order.  So after every batch the whole corpus is run again:
`differ` must be 0 and no `agree` may drop before the commit.  A difference found this way is
either ported (the usual case — each was one Rust function) or refused by name.

## The loop (`selfhost/front/elab_probe.py one | where | fam` carries the three probe scripts)

- **A development build of the elaborator** — `loft --native-c --out <dir>
  selfhost/front/elab.loft` (no `--ship`): about 7 minutes at load 4, half the optimised one.
- **One program, both reports diffed** (`one.sh`): `LOFT_EMIT_PARSED=<img> loft <f>`; `defs <img>
  program <std>` against `<elab> <img> <f> <std>`.  `defs`, `std.store` come from a
  `LOFT_ELAB_KEEP=1` gate run.
- **`LOFT_ELAB_WHERE=1`** (new, `elab.loft`): the source LINE each refusal was first met on, to
  stderr.  `where.sh` prints that line; `fam.sh "<refusal text>"` collects the lines behind one
  refusal over the corpus — the first thing to run on a family.
- **`LOFT_ELAB_DEFS=<exe> LOFT_ELAB_BIN=<exe>`** (new, `elab_gate.py`): a corpus run with tools
  built before — about four minutes.  `--check` / `--write-baseline` refuse them and build their
  own (about 25 minutes): that run is made once per commit, after it.
- **`LOFT_ELAB_VERDICTS=<file>`** per run; two files diffed name the verdict changes.
- **`LOFT_TRACE_MINT=1`** on the Rust side names the call chain that minted a store type — it
  settled a row-order question in one run where three probes had not.

Order of a batch: `fam.sh` → a probe in `scratchpad/p/` hand-checked against `loft --interpret`
→ Rust's report read for the shape → the Rust function read (never guessed) → the port →
development build → probe → corpus run → cell → commit → real gate → baseline commit.

**Traps met.**  The gate's own build reads `selfhost/src` in its first two minutes: no edit
there until its `loft --native-c --ship` process is older than that.  A loft local bound to
a function's answer can be a VIEW of the argument (`tail = el_ir::unspan(bl.operators[-1])`):
read what you need from it BEFORE the statement it views is rewritten.  The first pass often
answers `NdNull` with a type — a test of the CODE's shape there must ask the syntax instead.

## Done this session (commits 45d19cd75 … d874c229c)

Admitted: a vector parameter rebound by a literal and from a vector; a struct-enum parameter
rebound; `&text` written whole; a record into an element at an unproven index; `return null`
from a text function; formatted variant records and vectors of records; a field or element
handed to a `&` parameter; a vector `if` tail answering parameters; `!` (postfix) on a struct
enum; a literal rebinding a local it reads.

Rust rules ported because a lifted refusal showed the port differing: `@FR-E-Left`
(`fix_place_before_value`); the block-closing line rule (only a `;`-ended last statement moves
the line to the `}`); `parse_var`'s `into` arm asked of the exact type; a view answered through
`x?` published on the first pass; `vector<T>` definitions at a parameter's body entry and at a
declared comprehension; format-hole rows on both passes; `c ?? Rec {..}` borrowing what its
subject's read borrows; a `-> E?` body over a variant tail; a `w!` match subject's borrow.

## Later the same session (commits 56f990798 … fc10743c3)

`null` into a declared local of any type; prefix `!` on a record and a struct enum; `return
null` and a literal tail for `vector<T>?`, `return null` for a struct-enum `E?`; a tuple bound
to a declared tuple of other scalar member types (`tuple_convert`); `vector<integer?>`,
`vector<float?>`, `vector<boolean?>` (stored as the scalar's vector — the base
`vector<float>` row on the first pass, its definition on the second); a keyed local returned,
a keyed collection bound from a call with scalar arguments, a keyed literal at a tail
(`__kvb_N`); `destructure_constant_member`.

## Evening of the same day (commits a2961bce2 … 16caf172d): 866 of 2164, 0 differ

A constant-fill comprehension; `vector<S?>` as a whole area (`e2_nullable_elem`: the tagged
`__nullable<S>` minted where the type is read — literal items in a local, a struct's field and
a field append, the bind's `read_through_tag`, the loop element, `heap_null_test`'s tagged arm,
the element write); a tagged slot as an `S?` argument; a vector literal mixing `S { … }` with
other items of `S`; a record `if` tail whose arms are an element of a vector parameter, a
fresh call (`join-arm-owner`) or a call answering a view.

**The owner asked about the pace** (four programs a run): the answer given was to port by
Rust RULE and not by refusal, batch several ports per build, and run the real gate once per
several commits.  `vector<S?>` done that way was six builds for five corpus programs — the
tail is long whichever way it is cut, but the area is whole and its next refusals are other
element types.

**The interpreter runs the elaborator**: `loft --interpret selfhost/front/elab.loft <img> <f>
<std>` answers in three seconds once its cache is warm, 4m40 after an edit — no faster than
the 7-minute development build for a changed tree, but a second probe costs nothing.

**Later still (commits 62d05d2e9 … the `vector<i32>` commit): 879 of 2168, 0 differ.**
`ref_return`'s copy leg for a branch tail (`materialize_return_into`: whole, and arm by arm
where an arm answers null), `arm_join_type` filtering MINTS only, a struct enum beside `null`,
the unit variant before a `{`, and `vector<i32>` — the best yield of the day (seven programs),
because 32 programs met it and its layout is one small table of ops.

**What paid, measured over the day**: an area many programs MEET (an element width, a tagged
element) admits five to seven; a single refusal's own shape admits one or two.  Rank by the
programs that meet a family, then read its source lines (`elab_probe.py fam`) before choosing.

**Night (to 12ca00191): 890 of 2172, 0 differ, the real gate's own figure.**  `u32`, `i32?`
and wide-literal vectors; a function-valued struct field (`fn_number_write`,
`fn_ref_field_read`, `fn_call_tmp`).  By formal rule: 101 of the suite's 459 rules agree on
every program, 165 on some, 193 on none (`rewrites` 55 of 107; closures, interfaces,
operators, tuples and capabilities none whole).

**A call through a function value prints its temporary's variable NUMBER** (`fn_ref[18](…)`):
the one place the reports compare the ORDER variables were made in.  A temp Rust mints on the
first pass and the port only on the second agrees by name and differs there — read Rust's order
with `LOFT_VAR_TABLE=<fn>` and mint on the first pass too.

**Next: function values**, where the refused programs cluster — a lambda's capture of a
function (11), of a vector it writes (11: the closure shares it as a read one does; the body's
`c += [x]` is `OpAppendVector(capture, {Vector …}, 0)` with the push aimed at the capture —
probe `scratchpad/p/cw1.loft`), of a keyed collection (21), of a tuple or a link; then the 51
programs whose call through a local is refused only because its lambda was.  loft3 took
605f3d2f5 as this branch's cut for the next PR and fixed loft#2520 on `tuxedo-pr2518-next`.

**To 6198dab69: 898 of 2175, 0 differ** — a lambda appending to a vector it captures, a whole
value written through a link (`copy_ref`; nullable scalars, text, enums), a scalar link read
under `??` and `+`.

**The next rule-sized piece: `boxed_tuple_return`** (35 programs meet "a call returning a heap
value: TyTuple", 73 "a tuple with a member of this type").  A function answering a tuple with
a heap member answers a `__tuple<…>` RECORD: its declaration is rewritten (hidden `__retbuf`,
`-> __tuple<S,integer>`, the definition at the return type's position), its tail is a
`synthetic_tuple_return` block building into a `__ref_N` (a record member copied into
`OpGetField(buf, pos, kt)`, a scalar set), and a destructure of the call holds the answer in a
`__ref_N`, reads a scalar member and COPIES a record member into a work ref the place then
names.  Rust sites: `Parser::boxed_tuple_return` (definitions.rs 3539, 4751, 8433; vectors.rs
1896, 2082, 2329; generics.rs 268; control.rs 2415, 5279, 5408; call_args.rs 1220).  The port
does NOT box the callee today and nothing refuses it — it is hidden only because every caller
is refused — so port the callee FIRST.  A caller-side draft (the destructure) is kept in
`scratchpad/boxed-tuple-destructure.patch`; probe `scratchpad/p/tp1.loft`.

**A build reads the sources in its first minutes — every time.**  Twice more this session an
edit landed while a run's build was under a minute old (a development build at 28 s, the real
gate's at 35 s); both runs were discarded and repeated.  Check the build's age in ONE command
and edit in the NEXT, never in the same one.

**All layers measured at this tree's suite pin (508580956), 2026-11-17** — into scratch
baselines, NOT committed: the committed `bgen_survey` / `run_image_survey` baselines are still
pinned at 38d4877cb and a `--check` refuses to run against them.

| Layer | Reading |
|---|---|
| front end (gate E), real gate | 924 of 2187 agree, 0 differ, 1263 refused (b10ad3b02) |
| writer, plain programs | 9091 of 9093 functions identical, 0 differ; 1316 of 1317 streams |
| writer, test functions | 5756 of 5758 identical, **2 differ**; 2616 of 2660 streams, 3 differ, 41 natives tables |
| runner on Rust's code | 1316 of 1317 programs; 3967 of 3978 test functions (7 stopped, 2 fault, 1 noimage) |
| runner on loft's code | 1315 of 1317 programs; 3966 of 3978 test functions |

**The writer defect to fix before those baselines can be committed.**  `t_4File_lines` and
`t_4text_char_slice` are written with a frame 16 bytes larger than the image's (first op
`41 176 0` against `41 160 0`) in three test programs: `E-Left/2503…::test_each_pass_reads_the_
vector_before_its_write`, `E-Left/an-operand-is-read…::test_every_scalar_type`,
`E-Null/the-null-pattern…::test_the_float_character_and_boolean_patterns`.  Rust's two images
are stable across emits.  With `LOFT_NO_TEXT_LEN=1` on both emits `char_slice` agrees; with
`LOFT_NO_DISCHARGE_INTO=1` `File.lines` agrees — so the writer's mirrors of those two rewrites
decide differently from Rust's for these library functions in these programs.  Repro: the test
file with `fn main() { <test>(); }` appended, emitted from the file's own directory with
`LOFT_NO_WARNINGS=1 LOFT_NO_TEXT_BORROW=1` (`--emit-image`, `--emit-run-image`), then
`bgen_check <store> <run> --dump t_4File_lines`.  What is measured so far for `File.lines` (the rewrite's own trace: `discharge-into:
t_4File_lines — 1 read(s) straight into their local`, i.e. `c = self.content() ?? ""` loses
its `__ncc_1`): the program image holds `__ncc_1` at no slot (65535) and the locals after it
24 bytes lower (`p` 120, where the rewrite switched off gives `__ncc_1` 120 and `p` 144) — so
the image IS the rewritten function, and the writer still lays the frame out 16 bytes deeper
(every stack distance in the first operators reads 176 where the image has 160).  The first operator IS the entry
`reserve_frame`: 160 in the image, 176 written.  Frame start is 40 (the two arguments and
the return slot), so Rust reserves to 200 — the end of `e` (192 + 8) — and the writer to 216,
the stepped end of `_elm_3` (200 + 12), the true extent.  With the rewrite off both reserve
alike.  Both sides compute the same thing — the largest slot end over the
placed locals (`Function::frame_hwm`, `bgen.loft` 2879) — so Rust's 200 means `_elm_3` had
NO slot yet when its entry reserve was written, and took one during code generation; the
image, dumped afterwards, shows it at 200.  Nothing in the image tells a slot placed up
front from one placed late, so the writer cannot follow.  Likely a compiler defect (the
`discharge-into` and `text-len` rewrites leave a local unplaced at entry, against "one
function-entry reserve covers every local's slot"), harmless at run time if the first set
reserves for it — confirm in `src/state/codegen.rs` (the first-`Set` helpers) and either fix
the placement or carry a flag in the image.  The question that was open:
which variables Rust's entry reserve covers once the rewrite has
moved the slots down — read the entry reserve in `src/state/codegen.rs` against
`bgen.loft`'s before changing either; Rust under-reserving there is one possible answer, and
then it is a compiler defect to file, not a writer one.  A
scratch tool that prints an image's stack positions is a ten-line loft program over
`load::load` (the fields are private to `ir`, so it runs in a scratch copy of the package).
A development build of `bgen_check` needs
`LD_PRELOAD=libm.so.6`: its `crt` shim is linked without the maths library there
(`undefined symbol: trunc`) — a build defect of its own, not filed.

**Owner, 2026-11-17: build bigger chunks, test specific things, fewer full runs.**  Several
ports per batch, each checked by a probe on the interpreter-run elaborator (seconds once
warm), then ONE development build and corpus run per batch and one real gate per committed
batch or two.  A batch run shows several differences at once; most are gaps the lifted
refusal uncovered, not the new ports.

**Owed after the next PR lands (loft3's union on `tuxedo-pr2518-next`, suite pin b2194ff):**
gate E reads 886 of 2185 there with 2 DIFFER, both variable numbering, committed as `differ`
rows in that branch's baseline — cure them in the elaborator after rebasing:
1. `rules/matching/M-Match/a-match-subject-from-a-later-function-keeps-its-own-temp.loft`
   (loft#2520's cell): Rust mints one more `__work` text temp (`__work_3` where the port has
   `__work_2`).  Compiler change c10d11b64 names a match subject temp by its SITE (pass 1's
   number per site; a site pass 1 did not mint at is `_match_subj_late_N`) — mirror it.
2. `tests/scripts/a-captured-view-of-a-call-result-is-released-once-past-an-early-exit.loft`:
   the closure build's function type reads `deps { items: [10] }` in Rust, `[9]` in the port.

## Open, in order

1. **Out-of-line children** (`@FR-L-OutOfLine`) are carried in their first increments — layout
   (`child_rec<S>`, struct and struct enum), read, literal child, whole write, the demote
   (`h = N { next: h }`).  Still refused by name: `move_child` (`t.left = t.left.right`),
   `write_back_call` (`t.left = insert(t.left, k)`), a `&` to a child, a child given a value
   that is no literal, local or fresh call.  The notes below are the probe that started it.
   (Before: the largest family, 75
   programs meet "a type whose fields contain it again", 64 "an omitted field of this type").
   Rust's shape, from a probe: the field's store type is `child_rec<Tree>` (`ChildRec(100)`,
   4 bytes); a read is `OpRefFromChildRec(OpGetField(t, pos, type))`; an omitted child is
   `OpSetChildRec(field, OpNullRefSentinel(), type)`; a literal's FIRST child literal is built
   in place (`__ref_p2_N = OpChildRecInPlace(field, type)`, then its fields), a later one apart
   in an "Object" block handed to `OpSetChildRec`.  Read `typedef.rs` / `objects.rs` for which
   child is built in place before porting.
2. Next by blocked programs: a vector literal mixing struct literals and other items (33), a
   slice of a vector that is no local of plain scalars (27), an index that is a range, a key
   list or a limit (20), `vector<S?>` / `vector<text?>` / `vector<u8?>` (41, each its own
   layout), a keyed bind from a call with a record argument (`protectable_ref_args`).
3. **A loft language defect met in the port itself: loft#2520** (filed, not fixed).  In
   `el_ex_place::fields_at`, a `match ol! { .. }` over a `ir::Node?` local bound from
   `out_of_line_init`, in a branch that never runs, corrupts the sibling local `at`: the
   interpreter faults in three seconds, the C backend answers wrong.  `on = ol!; match on`
   and a helper function (`el_ir::empty_insert`) are both fine; the issue holds the boundary
   table and the recipe.  No standalone reduction yet — the elaborator is the repro.  Until
   it is fixed, do not `match` directly over a `x!` unwrap in the port.
4. Known differences OUTSIDE the corpus, found by probes and not yet fixed: `R { c: [] }` with
   `c: vector<boolean>` mints the `vector<boolean>` row on the first pass where Rust does not
   (`LOFT_TRACE_MINT` on both sides will name it).
   Also: `q = same(P { a: 2 })` with `fn same(p: P) -> P { p }` — Rust types `q` without the
   literal's own `__ref_p2_N`, the port lends it (the buffered-call arm of `call_dependencies`).
3. Families probed and left: a `text?` field or element as a `&text` argument (Rust calls a
   store-text twin `n_f@st1`); `vv[0] += [9]` on a vector of vectors (does Rust mint `_elm_N`
   there?); a vector tail that is a recursive call arm, `return q ?? [..]`, `return a + [b]`;
   `text? + text` (an "Add text" block under `OpConvBoolFromText`); a record literal written
   into a field or an element (four sub-refusals, each its own shape).
4. Everything in `handoff-2026-11-16-day.md` § Open that is not above (the writer/runner
   baselines, `store_records.rs` over the ceiling).
