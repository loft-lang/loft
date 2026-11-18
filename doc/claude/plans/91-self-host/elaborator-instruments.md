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

## 3. How much is ported — route coverage over the corpus

No new mechanism: count the `route` lines.

- `routes.tsv`, one row per handler: programs where Rust takes it, programs where the port
  takes it at the same position, programs where the port refuses at that position.
- The headline the handoff carries beside the agree count: "N of M routes the corpus
  exercises are carried in every program that takes them".
- Sorted by "refused" it is the ranking of what to port next, in Rust's own terms.

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
