# @PLN91 — the refusals, ranked: the porter's worklist

**Where this starts:** parity holds (parity-worklist.md): every program that agrees on its IR
agrees on its contract, state and reasons too, and an `agree` requires all four.  What is left
is the refusals.

**Measured at** try dc356f49d (porter tip 075d704de, 2026-11-26): 1634 of 2269 agree, 634
refused, 0 differ; 2396 refused definitions and 4538 beside a refusal (1453 of them numbers
only).  Data: `kr-tools/try-dc356f49d/{verdicts,where}.tsv`; re-rank a later try with
`kr-tools/family.py TRY PATTERN…` (a topic's programs admitted, and those one refusal short).

**The landscape is flat.**  184 refused programs meet one family, 185 two, 265 three or more;
361 families, 106 of them met by one program.  Clearing families greedily admits 4–5 programs
per family at first, 51 in the first 15, 69 in the first 20.  So the order below is by TOPIC
(the families one port clears together), each measured as "admitted once the topic is ported /
one refusal short of it":

| # | topic | admitted / one short | programs meeting | the families in it (programs) | port site |
|---|---|---|---|---|---|
| T1 | a record answered from something other than a literal (was R7) | 10 / 5 | 29 | a function returning a record from something other than a literal (21: `if` 11 defs, `call OpGetRecord` 7, `block match` 5); a `return` of a record that is not a literal (`other` 7, `if` 4, `block EnumUnitLit` 4); a value block answering a record | el_expressions.loft:827, el_ex_null.loft:3261 |
| T2 | `par` workers and loops of a non-scalar | 7 / 1 | 11 | a `par` worker answering TyOptional (10 defs), TyVector, TyEnum; a `par` loop over TyTuple, TyOptional | el_par.loft:99, :273 |
| T3 | tuples | 28 / 30 | 119 | 93 refusal kinds: a member of a boxed tuple that is no plain scalar, text or record (18); a tuple member of this subject: TyTuple (18); a vector of tuples with a member of this kind (16); a tuple with a text member that is no literal or variable (15); a boxed tuple answered by something other than a tuple literal (24 programs over 7 tails) | el_ex_tuple.loft:138, :526, el_expressions.loft:341 |
| T4 | links: `&` and `reference<…>` (was R5) | 19 / 19 | 81 | a `&` reference to something other than a scalar or a vector place (25); a `&` argument that is not a local (12); a link re-pointed at a place of another type | el_ex_vector.loft:1599 |
| T5 | lambdas and captures | 18 / 12 | 85 | a lambda that captures a local of its function (17); a lambda answering a record or a collection beside a shared capture (9); a short lambda with no `fn(…)` its position expects | — |
| T6 | generics (was R4) | 21 / 13 | 70 | a type with arguments, keys, a limit or a member (18: `Tree`, `Bag`, `B` 9 defs each, `Wrap` 8); a generic instance at text whose body holds an answer other than a parameter (12 hidden); an operator on a type variable its bounds do not declare | el_types.loft:685 |
| T7 | a vector answered from a tail (was R3) | 7 / 6 | 43 | a function returning a vector from this tail (25: `null` 18 defs, `block Append Vector` 8, `an owned local` 8, `block block` 6); a `return` with a value of this type (22); a function returning a keyed collection (8) | el_ex_deliver.loft:606, :657, :712 |
| T8 | MEASURED: no family of its own, a second refusal another one causes (§ T8) | — | 57 | a call that is not of a function by its bare name: a local of type TyUnknown (80 defs); a call returning a heap value: TyUnknown (49 defs) | el_ex_object.loft:1848, el_ex_call.loft:1990 |
| T9 | record literals (was R8) | 3 / 16 | 83 | a literal of something other than a declared struct (25 programs, 63 defs); a struct field of this type (26); a field the struct does not declare (18); a value struct or a struct template (16) | el_ex_into.loft:101, el_ex_object.loft:432 |
| — | a vector match over this element type (was R6) | — | 7 | 43 defs: a narrow integer element, TyTuple, TyOptional | el_ex_match.loft:1412 |
| R9 | the definitions a refused construct never registers (tester) | — | 157 | 1689 Rust-only and 108 port-only definitions; 1453 beside by numbers only (2339 at 635b29b75) | per site, below |
| R10 | the 32-bit regime's lowering | — | 0 | built (d067d87a8) | el_definitions.loft:92 |

**Recommended order:** T1 and T2 first: the most programs admitted per refusal kind, and T1 is
the porter's current topic.  Then T3, tuples, as ONE topic: by far the most programs met (119)
and the most admitted (28, and 30 more one refusal short), but 93 kinds, so it is cut into its
families and each try measured.  T8 is not a lever: its refusals follow from others (§ T8), so it leaves the ranking.

**Cleared since 635b29b75:** R1 (names across a used library: 2 programs and 11 definitions
left, `a method call`, plus the lib constants `CRYSTAL_HALO_K` / `CRYSTAL_CHUNK_SHIFT` as `a name
that is not a variable`) and R2 (heap constants: 1 definition left).  Their sections moved to
refusal-worklist-history.md.

**Probe programs, the most refused definitions of the family first:**
- T1: tests/scripts/2561-a-record-copied-out-of-a-view-is-absent-where-its-source-is (8),
  a-tail-match-answer-materialised-into-a-work-ref-is-freed-once (4 + 4 `return`),
  2488-a-branch-that-can-leave-copies-each-arm-into-the-return-buffer (3).
- T2: tests/scripts/1948-a-par-worker-may-answer-null (9), L-Null-Text/null-text-through-a-par-result.

The sections below name what Rust does for each older item (the routine to port, under the
corresponding-routine ruling); their counts are those of 635b29b75.

## T8, measured (2026-11-26, try dc356f49d)

The two TyUnknown families are knock-on refusals: the type is unknown because something earlier was
refused.  They are not a pass-1 typing gap.

**A call of a local of type TyUnknown (80 sites):** 78 of the 80 follow an earlier refusal in the
same definition, 70 of them of the lambda bound to that local ("a lambda that captures a local of
its function" 33, "a lambda answering a record or a collection beside a shared capture" 29, "a
short lambda with no `fn(…)` its position expects" 8).  The refused lambda leaves its local
untyped, so the call of it on the next line is refused too.  The definition is already refused,
so clearing this changes no verdict and no definition count: these sites belong to T5.  Probe
(both refusals on line 2; drop `q();` and only the lambda's remains):
`struct Rw { k: integer, v: integer }` /
`fn run() -> integer { m: hash<Rw[k]> = [Rw { k: 1, v: 11 }]; q = fn() { m = [Rw { k: 9, v: 99 }]; }; q(); len(m) }`.

**A call returning a heap value: TyUnknown (49 sites):** 33 have no earlier refusal of their own.
In every one of them the CALLEE is refused in a way that leaves its return type unresolved:
- 13: a generator whose element type the port refuses at the signature ("an iterator of an element
  other than a plain scalar, a text, a record or a tuple of those", line 0).  Probe:
  `fn gen(n: integer) -> iterator<vector<integer>> { for i in 0..n { yield [i * 7, 31]; } }` and
  `fn main() { t = 0; for v in gen(3) { t += len(v); } println("{t}"); }` (`main` is refused only for
  the call);
- 6 + more: a record named in the callee's signature and declared BELOW the callee.  Probe:
  `fn mk(n: integer) -> X { X { a: n } }` / `struct X { a: integer }` /
  `fn main() { x = mk(41); println("{x.a}"); }`: `mk` is refused as "a literal of something other
  than a declared struct", and `main` for the call;
- the rest: a generic type (`a type with arguments…`) or a callee whose own call is refused the
  same way.
A callee refused in its BODY, with a return type the port resolves, does not cause this: a
`vector match over TyTuple` in `fn mk(n: integer) -> X` (X above it) leaves its callers elaborated.
So register-then-refuse already holds for signatures; the gap is what the port cannot RESOLVE.

**Where that points:** T9's "a literal of something other than a declared struct" is mostly the
same cause: 29 of its 63 definitions name a struct declared below the literal.  The port has
Rust's forward stub (`claim_declared_type_names`, `DT_UNKNOWN`, adopted in place by
`parse_struct`, el_definitions.loft ~635), but `literal_def` (el_ex_object.loft:336) accepts only
`DT_STRUCT` / a record variant, so a literal met before the struct's declaration (pass 1 reaches
the function before the struct) is refused.  Rust's routing (objects.rs ~3064) has the same
`Struct | EnumValue` condition, so how Rust's pass 1 treats a literal of a stub is the routine
to port.  To confirm on a pass-1 trace before porting.  Porting it clears these T9 definitions
together with the callers refused for the call.

## R3 — a vector returned from a tail

**Sub-kinds:** `null` 36, `if` 15, `block Append Vector` 8, `an owned local` 7.
**Rust:** `ref_return` delivers each kind of vector tail into the caller's buffer
(`@FR-O-Buffer`): a `null` tail as the empty vector, an `if` arm by arm.
**Probe:** Col-Concat a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-
stands (13), 1493-a-branch-arm-yielding-a-locals-field-delivers-a-copy (13),
2233-a-coalesce-tail-delivers-into-the-callers-buffer-a-lambdas-too (7).

## R4 — a generic struct or enum

**Sub-kinds:** `Tree` 9, `Bag` 9, `B` 9, `Wrap` 8 (a type written with arguments, `Tree<T>`).
**Rust:** the type template and its instances (`@FR-G-Mono` for types).
**Probe:** a-template-that-names-itself-at-its-own-variables-compiles-on-its-own (18),
2547-a-generic-enum-named-above-its-declaration-is-laid-out (9),
a-library-carries-generic-types-across-the-boundary (8).

## R5 — a `&` to a place that is no scalar or vector

**Sub-kinds:** `TyUnknown` 28 (a place pass 1 has not typed), `TyFunction` 19, `TyHash` 14,
`TyTuple` 13.
**Rust:** `@FR-B-Ref-Uniform`: a `&τ` of any type is a link to the place.
**Probe:** a-link-to-a-fn-value-is-a-link-like-any-other (36),
1836-a-link-to-a-nullable-place-writes-through (10),
a-whole-write-through-a-link-to-a-plain-parameter-rebinds-the-callees-name-only (9).

## R6 — a vector match over this element type

**Sub-kinds:** `TyInteger` 24 (a narrow integer element), `TyTuple` 10, `TyOptional` 5.
**Rust:** `parse_vector_match` over any element, its reads at the element's own width.
**Probe:** a-narrow-vector-element-keeps-its-width-in-every-operation (23),
a-match-streams-an-iterator-of-any-element (8),
a-pattern-capture-takes-the-type-its-pattern-matched (6).

## R7 — a record returned from a tail that is no literal

**Sub-kinds:** `if` 24, `call OpGetRecord` 7, `block match` 6, `block scalar_match` 3.
**Rust:** `ref_return` / `MaterializeView`: each exit answering a view copies into the buffer.
**Programs:** first in the freed-in-turn ranking — alone it frees five.
**Probe:** 2561-a-record-copied-out-of-a-view-is-absent-where-its-source-is (8),
a-tail-match-answer-materialised-into-a-work-ref-is-freed-once (4),
each-pass-frees-the-answer-it-copied (3).

## R8 — a literal of something other than a declared struct

**What fails:** a struct literal whose name resolves to no struct the port has declared — a
record declared below its use, a library's, an alias.
**Probe:** 947-feature-worked-examples (9), 03-crystal-incr (7),
2497-a-return-naming-a-record-declared-below-is-that-record (5).

## The rule: register, then refuse

**A refusal never drops a definition Rust registers.**  Wherever the port refuses a construct
that registers definitions in Rust — a lambda and its closure record, a vector type, a function
whose default it cannot lower, a constant — it registers them first, as Rust does (same name,
same kind, same place), and then refuses; nothing of the construct is elaborated.  A definition
dropped instead shifts the number of every definition after it, so they all differ in numbers
only and no correct port of them can show.  The rule binds every new refusal site.

**Measured:** the gate prints `definitions one side registers: N Rust only, M port only` and
writes them per program (`LOFT_ELAB_ONE_SIDED_FILE`); a try's result line names any Rust-only
definition new against its base (`NEW RUST-ONLY DEFINITION(S)`, `one_sided.tsv`).

**A port-only definition is forbidden (owner, 2026-11-26): it is the port creating a new language
instead of porting one.**  A definition only the PORT registers, new against the try's base, makes the try `TRYRESULT issues` (`NEW PORT-ONLY
DEFINITION(S)`, listed in the report and in `port_only_new.tsv`): the port minted something Rust
never does.  It is a ratchet against the base, so the existing port-only rows (85 at try
8e161452b) do not block a promotion; they are fixed first, before any other noise pool.
Falsified on two past tries: e0c9090b9 against ec2d7a451 counts its 7 `vector.filter` instances
(it read clean before), 4d83248cc against 65c3397d1 counts 0.

**Order of work:** before an R-item starts, the tester aligns the numbering in the programs it
touches (R9), except where the R-item itself is the cause (R2's constants, R4's templates): then
the item's port shows as definitions won, not hidden behind shifted numbers.

## R9 — what a refused construct never registers (tester)

**Standing (owner, 2026-11-26):** between the porter's tries the tester keeps shrinking the diff
of refused programs: every difference in a refused program that is not the refused construct
itself, whether a knock-on refusal, a definition one side registers, or numbers shifted by a
dropped definition.

**The noise pools (2026-11-26).**  LANDED (try 438fea34d on 77258ed46, clean, 3 programs
refused → agree: `engine_host/tests/local_kernel`, `process/tests-unix/tree`,
`process/tests/start`): a package's own name and a lock-pinned dependency in a `use`
(`probe_own_package`, `resolve_pinned`), a packaged library in a `--lib` directory
(`probe_flag_lib_dirs`), and generic registration: type instances, their layout, and a refused
function instance's signature (generic-instance-registration.md).  Rust-only definitions
1689 → 800, unscored state rows ≈15800 → 6087, hidden differs ≈1631 → 1447.  Left:
1. Generic function instances Rust mints from a FILLED body (`instantiate_nested_generics`):
   LANDED for a non-standard-library template — at a body the port refuses to fill, the nested
   instances are minted from the template's body with the binding applied (try 65c3397d1).  A
   standard-library template's call is skipped: Rust's fill builds `filter` / `map` in place.
   The rest follow the porter's body filling (T6).
2. The roads of a `use` inside a package: LANDED, every road of Rust's `lib_path` in its order
   (`package_lib_path`: path dependencies, the shadow guard, the sibling package, the run's
   `lib_dirs`, a module beside the file).  No corpus program reaches the new ones; five probe
   packages (scratch, one per road, hand-computed answers 42/37/11/5/8) agree on both front ends
   and are refused by the elaborator before it.  Still refused where a road the port does not
   carry could answer: `LOFT_LIB`, an installed package, auto-install, the registry's newest.
3. Fixed by the porter (b5ad08833): the `main_vector<single>` pub items, and the library's
   `main_vector<Pair>` minted again from the main source (H-Drop/2436, 17-libraries, 188 agree).

A construct the port refuses before it registers its definitions shifts the number of every
definition after it, so those definitions differ in numbers only (`beside numbers`).  The fix at
each site is to register what Rust registers, then refuse.  Done: the refused lambda
(7201cfb57: 277 definitions); an unnamed vector element type, a refused parameter default and an
expected empty literal (6711b88de: 84).  Next, by the shifts each causes:
- a vector of an element type the port refuses, which never registers its `vector<…>` (102);
- a named argument (86, one program);
- a vector of tuples with a member of an unsupported kind (52);
- the remaining one-side definitions: a user function (292), a generic template (280), a
  struct (121), a drop helper (90), a tuple record (84).
`np/shift.py` and `np/verify.py` (the tester's scratch tools) rank them from the saved IR.

**Rust-only types (open).**  52 at loft30-gate 12d1c0290 (vectors, tuple records, type
instances; Rust-only definitions 283), one to four per program.  What each open group needs:
- iterator element vectors (`vector<iterator>`, four programs): Rust's `check_vector` at the
  wrapper's field fill (`typedef.rs` `fill_database`, the `main_vector<iterator<…>>` place) on
  the SECOND pass; the port fills that wrapper on the first pass only, where an iterator field
  has no content type (`c = -1`), so it refuses before its `check_vector` arm.
- the 2501 module cycle (`BvcCx`): `Parser::resolve_deferred_unknowns` (re-applied imports, the
  deferred stubs) — no port twin.
- forward-reference `main_vector<unknown>` (2547, 2623): the literal-below test is positional;
  counting every stub as below lost 29 agreeing definitions in 947 — Rust's rule is narrower.
- a lambda argument's tuple record on the first pass (`last(v, |x| { (x, x + 1, x + 2) })`):
  making it there shifted later second-pass rows of the same program — needs Rust's exact moment.
- cross-library keyed tuples and instances (2435).

## R10 — the 32-bit regime

Built (d067d87a8): a `#width 32` file is lowered as the Rust front end lowers it — W-Name,
W-Lit, W-Count, W-Op, W-Call (both sides), W-Store, `@FR-I-Narrow-U32`'s guard, and the
`@FR-I-Join` integer union in `if_expr`.  Every regime program with a Rust image agrees; the two
`@EXPECT_ERROR` cells have none.  Not carried: the regime under `LOFT_WIDTH=32` for the standard
library, and the `match` arms' join (`join_arm_into`'s union and `arm_fits_named_width`), which
no corpus program reaches yet.
