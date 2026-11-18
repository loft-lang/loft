# The elaborator's instruments — seeing both passes and Rust's route (@PLN91)

Design by the porting agent, 2026-11-18, on the owner's request; built by the TESTER, one
commit per item, which records under each item what it built and measured (the form
[front-end-instruments.md](front-end-instruments.md) uses).  The elaborator's port continues
in parallel and starts using each instrument when it lands.

## The question

Gate E answers "is the elaborator's final image Rust's?" per program and per function, and a
refusal names where the port gives up.  It does not answer three things the port needs daily:

1. **What did Rust's FIRST pass leave?**  The two passes hand state to each other (a local's
   type, a function's result type, its borrows).  Only the image after the second pass is
   compared.
2. **Which Rust code handled the shape the port refuses?**  A refusal is named after the
   port's symptom, at the point the port stopped.
3. **How much of Rust's front end is ported?**  The agree count measures corpus programs,
   not rules.

## What was measured before designing

- The existing state trace (`LOFT_STATE_TRACE=1`, `src/parser/state_trace.rs`,
  `selfhost/src/el_trace.loft`, `selfhost/front/state_diff.py`) prints on both passes, at two
  points (`stmt`, `arg`), three channels: `expected`, `assign_target`, `cur_type_vars`.  It
  carries no variable type and no function result type.
- Two first-pass windows met on 2026-11-18, both invisible to every instrument:
  - `t = ["a"].map(shout)`: the only differ was the local's type, `vector<text["xs"]>` in
    Rust against `vector<text>`.  Rust's first pass answers the function's declared result
    with its borrow; the cause was found by guessing.
  - `copies = map(src, |r| { r })`: the same differ where the callee is a lambda.  The port's
    first pass has not derived the lambda's borrow yet.  It is refused by name ("a `map` by a
    lambda answering a record that borrows its parameter") because Rust's first-pass state
    could not be read.
- A wrong name held a whole family: 10 programs carried "a generic local over the type
  variable … a function-typed local".  Rust never instantiates a generic there:
  `parse_call` (`src/parser/control.rs`, the `"map"` / `"filter"` / `"reduce"` arms) builds
  the call in place.  The route was found by reading Rust's lowering by hand.
- Size of the question: `src/parser/` holds 163 `parse_*` functions; the port holds 651
  `el::refuse` sites and 72 `port:` citations.

## 1. The first pass, compared — two channels on the state trace

Extend the existing trace; add no second mechanism.

**New point `fn-end`**, printed once per function per pass, when its body has been read
(Rust: where `parse_function` leaves the body on each pass; port: after `finish_body` on the
second pass and at the matching place on the first).  One line per fact, so a diff names it:

```
state <pass> <line>:<col> fn-end <fn> ret=<type>
state <pass> <line>:<col> fn-end <fn> var <nr> <name>=<type>
```

- `<type>` is the spelling the `defs` report already prints (`vector<text["xs"]>["__vdb_4"]`),
  so both sides share one printer each and deps are visible.  Position is the function's
  header.
- Variables in number order: the mint order is itself a fact the port must reproduce.
- A lambda prints as its own function (`__lambda_main_0`).

**New channel `value=<type>` on the existing `stmt` point**: the type the statement's
expression answered, on both passes.  This is where a first-pass answer of "unknown" against
a real type shows at the statement that made it.

**The gate reads it** — a new layer in `elab_gate.py`, `--pass1`:

- For every program that AGREES, diff the `state 1 …` lines of both sides.  Verdict
  `pass1-agree` or `pass1-differ <first differing line>`.  An agreeing image over a differing
  first pass is a latent differ: the next construct that reads that state will differ.
- For a REFUSED program, compare only the functions the function layer already calls
  agreeing.
- A baseline by function, as `elab-functions` has; a `pass1-differ` does not fail the gate
  while the count is being driven down (a ratchet: the count may only fall).
- Positive control: one cell where the port's first pass is made to answer a wrong type on
  purpose (an env switch in `el_trace`) must read `pass1-differ`.

Expected first reading: many `pass1-differ` rows on agreeing programs.  That list, grouped
by channel and shape, is the port's worklist of pass-order windows, replacing guesses.

### Built — step 1, the two channels on both sides

`fn-end` is as designed, on both sides: Rust prints it where a function's variables are handed
to its definition (`Parser::state_trace_fn_end`, four sites: `parse_function`, the two lambda
forms and `default_value_fn`), the port where `parse_function` stores them
(`el_trace::fn_end`), a lambda included.  Types are in the IR dump's spelling on both sides
(`Type::show`; `report::type_show_in`).

`value=` is a point of its own, `stmt-end value=<type>`, at the position its `stmt` line has
and printed after the statement — not a channel on `stmt`.  The type is not known where `stmt`
is printed, and holding that line back until the statement is read would reorder it against
the points of the statements nested inside it.

With the switch off nothing is printed and nothing else changed: gate E's verdicts on a sample
of 150 corpus programs are the ones of tip c62b395bb.

First reading, on 140 agreeing programs of that sample (`state 1` lines only): 30 agree, 110
differ.  By the kind of the FIRST differing line: 54 `stmt-end` (a dep the port has not derived
yet, `text["__work_1"]` against `text`), 24 `arg` and 16 `stmt` (`expected=` set in Rust and
not in the port — the two points that existed before), 14 `fn-end … var` (a variable minted in
another order or under another name, `e` against `e#iter_state`), 2 `fn-end … ret`
(`vector<integer>` against `vector<integer>["__retbuf"]`).

A differ the corpus does not hold, met while trying the trace: a `map` over a vector LITERAL
bound to a local (`t = ["a"].map(shout)` beside a later lambda) mints `_elm_1` and `_vec_1` in
the other order, on both passes; the image differs (`vector<text["_elm_1"]>` against
`vector<text["_vec_1"]>`).

### Built — step 2, the `--pass1` layer

`elab_gate.py --pass1` compares the `state 1` lines by FUNCTION: every function of a program
that agrees, the agreeing functions of a refused one.  A function's lines are those up to and
including its `fn-end` group, so a lambda's `fn-end` also closes what its enclosing function
printed before it; both sides are cut alike, and a difference there is reported under the
lambda's name.  The report ranks the kind of the first differing line with one example each;
`LOFT_ELAB_PASS1=<file>` writes every function's verdict and both lines.

The baseline block `elab-pass1` pins the functions that agree (`--pass1 --write-baseline`).  It
is a ratchet: a pinned function that differs is named (`PASS1 DROP`, also under
`--functions-against`), and the check fails only when fewer functions agree than were pinned.

The positive control runs in every `--pass1` run, not as a separate cell: one program whose
first pass agrees is read again with `LOFT_STATE_TRACE_CONTROL=1`, under which the port prints
`ret=(control)` on the first pass; the layer must read `pass1-differ` or the gate stops with
"the layer cannot fail".

Whole corpus at loft30-gate 5c83f4c82 (suite be55620c), 68 s with built tools: **8854 of 15866
functions agree on the first pass**, 7012 differ; by program 602 of 2085.  First differing
line: 2480 `stmt-end`, 2154 `arg`, 1236 `fn-end var`, 1038 `stmt`, 104 `fn-end ret`.  The two
points that existed before (`arg`, `stmt`: an `expected=` the port does not set on its first
pass) hold 3192 of the 7012.

## 2. Rust's route, joined to the refusal — the `route` point

**Rust side.**  One line when the front end takes a special route for a construct:

```
state <pass> <line>:<col> route <handler>
```

`<handler>` is the Rust function's name (`parse_map`, `parse_multiply`,
`try_generic_instantiation`, `materialize_return_into`, `classify_ret_promotion=Rename`).
Place it at the DISPATCH, not inside each handler, so the set stays small and each line says
which branch won:

- `parse_call`'s special-form arms (`control.rs`, the `match name` that holds `"map"`),
- the generic path (`try_generic_instantiation`, `bind_monomorph_join_return`),
- `parse_method`'s built-in arms,
- the return path (`materialize_return_into`'s whole-or-per-arm verdict,
  `classify_ret_promotion`'s verdict — already printed by `LOFT_TRACE_RR`; route it here),
- the literal paths (`parse_multiply`, the vector / struct literal entry),
- the match entry (record, vector, cursor, iterator).

Start with those six groups; add a group when a refusal family turns out to have no route
line at its site.

**Port side.**  `el_trace` gets `route(e, at, name)`.  The port prints the same handler name
where it takes the mirrored path, and `el::refuse` prints
`state <pass> <line>:<col> refuse <text>`.

**The join** — in the where-list the tester sends per tip, each refusal site gains Rust's
route lines at that line:

```
scripts/13-map-filter-reduce:130  refused: a `filter` of an element of this kind: TyReference
    rust: 130:10 route parse_filter
```

and the definition-level ranking gains a column "Rust route" (the handler most sites of the
family share).  A family whose sites share one handler is one port; a family spread over many
handlers is a symptom name and wants splitting.

### Built — step 3, the `route` point in Rust, `refuse` in the port, and the join

`Parser::state_route(handler)` prints `state <pass> <line>:<col> route <handler>` at the lexer's
position.  Five of the six groups are in:

- the special-form calls: `parse_parallel_for`, `parse_par_fold`, `parse_map`, `parse_filter`,
  `parse_reduce`, `parse_sort`, `parse_insert`, `parse_reverse`, `parse_reserve`, `parse_any`,
  `parse_all`, `parse_count_if`, and the arms written in place as `call:<name>` (`assert` and
  the log family, `deliver`, `expose` / `release`, `type_of`, `field_value`, `clear`, `next`,
  `exhausted`, `type_name`, `typedef`);
- the generic path: `try_generic_instantiation` (only where it found a template),
  `bind_monomorph_join_return`;
- the return path: `materialize_return_into=whole` / `=arms`, `classify_ret_promotion=<verdict>`;
- the literal paths: `parse_object`, `parse_vector`, `parse_multiply` (where it answers);
- the match entry: `parse_match`, then `parse_cursor_match`, `parse_scalar_match`,
  `parse_vector_match` or `parse_tuple_match`.

Two changes to the design.  A handler that is a function of its own prints at its ENTRY, not in
the dispatching `match`: that match has guarded and multi-line arms, the entry says the same
thing, and the change stays one line per handler.  And `parse_method`'s built-in arms are NOT
in: a method call is selected by `select_method_def` over an overload set, not by a `match` of
named arms, so there is no dispatch to mark; it is the group to add when a refusal family
shows no route at its sites.

The port: `el::refuse` prints `state <pass> <line>:0 refuse <text>` (it knows the line, not the
column).  `route` and `refuse` lines are left out of the first-pass comparison and of
`state_diff.py`: they say which code ran, not what a pass left, and the port's own `route`
lines come a path at a time (step 4).

The join: under `--pass1` the gate keeps Rust's route lines (`LOFT_ELAB_ROUTES=<file>`:
`program, pass, line, col, handler`); the tester's `where.py` prints them under each refusal
site and names the handlers a family's sites share, and `deflevel.py` carries the same as its
"Rust route" column.

Whole corpus at f260caf9b: 88474 route lines; the verdicts and the first-pass counts are
unchanged (1121 of 2085; 8854 of 15866).  By handler: `parse_object` 37232, `parse_vector`
36109, `classify_ret_promotion` 4851, `parse_match` 3394, `materialize_return_into` 1869,
`try_generic_instantiation` 1626, `parse_vector_match` 856, `bind_monomorph_join_return` 783,
`call:next` 508, `parse_scalar_match` 490, `parse_insert` 127, `parse_map` 112.  First joins:
"a slice pattern element of this form" — all 52 sites under `parse_vector_match` (one handler,
one port); "a record literal written into a field or an element" — `parse_object` 62,
`parse_vector` 15; "an operator with no builtin candidate" — `parse_object` 13,
`bind_monomorph_join_return` 6, `parse_match` 2 over 55 sites (a symptom name); the
definition-level families "a struct field of this type", "a value struct or a struct template"
and "a `&` parameter of a tuple" have no route at their sites: a declaration's route is the
next group to mark.

### Built — step 4, the port's `route` lines for three groups (by the tester)

`el_trace::route(e, name)` prints `state <pass> <line>:0 route <handler>`: the port knows the
line being read, not the column a Rust handler stands at, so the two sides are joined by
program, pass, LINE and handler.  Marked at the entry of the port's mirror:

| Rust handler | port function |
|---|---|
| `parse_map`, `parse_filter`, `parse_reduce` | `el_ex_match::map_call`, `filter_call`, `reduce_call` |
| `parse_sort`, `parse_insert`, `parse_reverse`, `parse_reserve` | `el_ex_object::sort_call`, `insert_call`, `reverse_call`, `reserve_call` |
| `parse_match` | `el_ex_match::match_expr` |
| `parse_tuple_match`, `parse_cursor_match` | `tuple_match`, `cursor_match` |
| `parse_vector_match` | `vector_match`, `stream_match` |
| `try_generic_instantiation` | `el_ex_call::instantiate` |

The gate's routes file now holds both sides and the port's refusals
(`program, side, pass, line, col, text`, side `rust` / `port` / `refuse`).  Whole corpus, the
sites Rust takes a handler at — both: the port takes it on that line; refused: it does not,
and refuses on that line; neither: no line of the port's there:

| handler | Rust | both | refused | neither | port only |
|---|---|---|---|---|---|
| `parse_match` | 3376 | 3313 | 8 | 55 | 0 |
| `try_generic_instantiation` | 1279 | 628 | 40 | 611 | 63 |
| `parse_vector_match` | 854 | 814 | 30 | 10 | 0 |
| `parse_insert` | 127 | 122 | 0 | 5 | 0 |
| `parse_map` | 108 | 94 | 6 | 8 | 2 |
| `parse_cursor_match` | 106 | 102 | 0 | 4 | 0 |
| `parse_tuple_match` | 64 | 58 | 6 | 0 | 0 |
| `parse_reverse` | 56 | 56 | 0 | 0 | 0 |
| `parse_filter` | 50 | 44 | 0 | 6 | 0 |
| `parse_reserve` | 49 | 42 | 0 | 7 | 2 |
| `parse_reduce` | 31 | 27 | 3 | 1 | 1 |
| `parse_sort` | 30 | 26 | 0 | 4 | 0 |

`neither` mixes two things the line join cannot tell apart yet: a call spread over lines (Rust
prints where its arguments END, the port where the statement is) and a path of the port's not
marked.  `try_generic_instantiation`'s 611 is the second kind: Rust instantiates from three
callers, the port's `instantiate` is one of several.  Not marked yet, each with its Rust count
in step 3's list: the literal paths, the return path, `bind_monomorph_join_return`,
`parse_scalar_match`, and the in-place call arms (`next`, `any`, `all`, `count_if`, …).

### Built — step 4, the remaining mirrors that are one place in the port

Marked where the port has ONE function or branch for the Rust handler: `parse_object` at
`el_ex_object::object_expr`; `parse_vector` at `el_ex_pattern::vector_value` and
`vector_build`; `parse_multiply` at `vector_build`'s `SRepeat` arm; `parse_scalar_match` at
`match_expr`'s scalar-or-enum branch; `call:next` / `call:exhausted` at `call_expression`'s
iterator branch; `bind_monomorph_join_return` at `fill_instance`'s `join_bind`;
`materialize_return_into` at `el_expressions::copy_arms` (the per-arm verdict only).

Whole corpus (programs: Rust takes it / carried; sites: Rust / both / port only):

| handler | programs | carried | sites | both | port only |
|---|---|---|---|---|---|
| `parse_scalar_match` | 101 | 96 | 482 | 469 | 140 |
| `call:exhausted` | 7 | 7 | 18 | 18 | 0 |
| `call:next` | 22 | 13 | 458 | 248 | 0 |
| `parse_vector` | 1608 | 644 | 29897 | 17941 | 311 |
| `parse_object` | 1635 | 234 | 25332 | 7681 | 1463 |
| `bind_monomorph_join_return` | 147 | 4 | 640 | 14 | 0 |
| `materialize_return_into` | 145 | 0 | 760 | 27 | 29 |

The headline is 2 of 30 (`parse_reverse`, `call:exhausted`).  What the numbers say about the
MARKS rather than the port: `parse_scalar_match`'s 140 port-only sites are plain-enum subjects
the port takes through its scalar branch and Rust elsewhere; a struct literal reaches the port
through more functions than `object_expr` (1463 port-only against 7681 shared: nested and
field-position literals), and a vector literal through more than the two marked; an instance's
join bind is filled where the instance is made, not on the line of the call Rust prints at; the
whole-copy leg of `materialize_return_into` has no single function.  Not marked, because the
port has no mirror to mark — it refuses, or takes the ordinary call: `classify_ret_promotion`
(its verdict is spread over the port's rename and bind paths), `parse_any`, `parse_all`,
`parse_count_if`, `parse_par_fold`, `parse_parallel_for`, and the in-place arms `field_value`,
`type_of`, `type_name`, `typedef`, `expose`, `deliver`, `assert`, `clear`.

## 3. How much is ported — route coverage over the corpus

No new mechanism: count the `route` lines.

- `routes.tsv`, one row per handler: programs where Rust takes it, programs where the port
  takes it at the same position, programs where the port refuses at that position.
- The headline the handoff carries beside the agree count: "N of M routes the corpus
  exercises are carried in every program that takes them".
- Sorted by "refused" it is the ranking of what to port next, in Rust's own terms.

### Built — step 5, route coverage

Under `--pass1` the gate prints the headline and the ranking, and writes the whole table where
`LOFT_ELAB_ROUTE_COVERAGE=<file>` asks (the tester keeps it per tip as `route_coverage.tsv`;
`routes.tsv` there is the raw lines of step 3).  One row per Rust handler, a verdict handler
(`materialize_return_into=whole`) under its function's name:

- `programs`: where Rust takes it; `carried`: the port takes its mirror at EVERY site Rust
  does in that program; `refusing`: at some site it does not and refuses on that line;
  `missing`: neither;
- `sites`, `both`, `refused`, `neither`, `port only`: the same by site (program, pass, line).

First reading at fcdbe9378 + 727c7bc75: **1 of 30 routes the corpus exercises are carried in
every program that takes them** (`parse_reverse`, 6 of 6).  Next closest: `parse_match` 375 of
399 programs, `parse_vector_match` 69 of 77, `parse_cursor_match` 11 of 12, `parse_tuple_match`
11 of 14, `parse_map` 12 of 17, `parse_filter` 9 of 12, `parse_insert` 6 of 8, `parse_reserve`
6 of 7, `parse_reduce` 5 of 6, `try_generic_instantiation` 59 of 145.

How to read a zero: a handler the port does not MARK yet reads as carried nowhere.  That is
the case for the eighteen handlers step 4 left (`parse_vector` 1608 programs, `parse_object`
1635, `classify_ret_promotion` 673, `bind_monomorph_join_return` 147,
`materialize_return_into` 145, `parse_scalar_match` 101, the in-place call arms), so their
`refusing` column — the programs where the port refuses on a line Rust takes the route on — is
the part that already means something: `parse_vector` 433, `parse_object` 409,
`classify_ret_promotion` 125, `bind_monomorph_join_return` 64, `materialize_return_into` 40,
`parse_scalar_match` 21.  Among the marked handlers the ranking by `refusing` is
`try_generic_instantiation` 17, `parse_match` 4, `parse_vector_match` 4, `parse_tuple_match` 3,
`parse_map` 2.  The headline moves as the port marks its paths (step 4's remaining groups) and
as refusals lift.

## What is deliberately not built

- No trace inside expressions below the statement (per operator): the image already
  compares that on the second pass, and the volume would hide the first differing line.
- No comparison of the first pass's IR: Rust's first pass mostly yields `Null` code by
  design; only types and tables carry over.
- No census of `port:` citations: route coverage measures the same thing from behaviour.

## Order

1. § 1 `fn-end` and `value=` on both sides, `state_diff.py` unchanged in form.
2. § 1 the `--pass1` layer with its positive control and baseline.
3. § 2 the `route` point in Rust (six groups) and `refuse` in the port; the where-list join.
4. § 2 port-side `route` lines, a group at a time, the porting agent adding them as it
   touches each path.
5. § 3 `routes.tsv`.

Items 1–3 touch Rust (`src/parser/state_trace.rs` and its call sites) only under the env
switch: the emitted image and every existing gate must be byte-identical with the switch off.
