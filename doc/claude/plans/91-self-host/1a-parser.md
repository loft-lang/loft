<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 1a — the parser in loft

Stage 1 of [§ The path](README.md#the-path--stages-and-gates): loft's front end written in
loft, its image byte-identical to the Rust front end's `--emit-image`.  This doc is the
design of the parser half (lexer, syntax, elaboration) and the order it is built in.

## The shape — three layers

```
source ─▶ C lexer ─▶ Toks ─▶ syntax (PEG) ─▶ Syn arena ─▶ elaboration ─▶ Data + IR (ir.loft) ─▶ image
          src/c/loft_lex.h    selfhost/src/syntax.loft      selfhost/src/elab*.loft
```

1. **The lexer is the C runtime's** (owner, 2026-11-01): `src/c/loft_lex.h` ports
   `src/lexer.rs`'s loft mode, reached through the `crt` shim (`crt::lex`) and decoded into
   `Toks` by `selfhost/src/lex.loft` — one `Tok` enum value per token, with parallel `line`
   and `col` vectors.  `src/lexer.rs` interleaves with the parser inside string holes (the
   parser sets `Mode::Formatting` / `Mode::Code`); the C lexer reproduces those switches
   itself, so the whole file is lexed once, before parsing.
2. **The syntax layer is PEG cursor matches** (@PLN35, `formal/matching.md`): one function
   per grammar rule, `fn rule(p: Syntax) -> integer?`, whose body is a `match p { [ … ] => … }`
   over the token cursor.  It builds a SYNTAX tree and decides nothing semantic.
3. **The elaboration layer is the port of `src/parser/`'s semantics**: it walks the syntax
   tree in source order, twice (the Rust parser's `first_pass` and second pass), and makes
   the same `Data` definitions, the same variable numbering and the same IR the Rust parser
   makes while it reads tokens.

**Why the split.**  A PEG rule may run and then be abandoned (an alternative fails after it),
so a rule must be pure: `(P-Atomic)` promises a failed attempt is invisible, and the purity
check (@PLN35 PC4) accepts only writes to the cursor and to records the rule builds.  The Rust
parser's semantics write shared state on every token — a definition registered, a variable
numbered, a type joined, a diagnostic added — so they cannot sit inside a rule that may be
backtracked.  Nodes a failed alternative built are garbage in the arena and nothing points at
them.

**The rule of the port still holds:** the same `Data`, the same variable tables, the same IR
and the same diagnostics.  The syntax tree is the one structure the Rust parser does not
have, and it is internal: nothing outside the front end sees it.

## The porting method — transcribe, do not re-derive

Above the syntax tree the elaborator is a TRANSCRIPTION of `src/parser/`, not a design of its
own: one loft function per Rust function, with the Rust name, its parameters in the same order,
its calls in the same order, and the same state (each `Parser` / `Function` field it reads is
the `El` / `Vars` field `scripts/parser_state.py` maps it to).

- **The tree stands in for the lexer.**  Where Rust peeks at or consumes a token, the loft
  reads the node that token starts.  Where Rust parses a span again — the second pass, or a
  lexer revert after an attempt (a literal tried in place, then built apart) — the loft calls
  the same function again on the same node.  The variables and work references the attempt
  leaves behind then fall out of the same control flow; they are not minted by hand.
- **Each transcribed function names its original** in its doc comment, one line:
  `// port: dispatch.rs dispatch_rank`.  A loft helper with no Rust original says so
  (`// port: none — <what Rust does inline instead>`).  `grep -c '// port: ' selfhost/src/el*.loft`
  counts what is transcribed.
- **A form loft cannot write** (a `&mut` into a collection, an early `?` out of a nested
  borrow) keeps the name and the call order, and the comment says what changed.
- **A gate E difference is located by function, not by image:** the state trace
  (`LOFT_STATE_TRACE`, `selfhost/front/state_diff.py`) prints function entry and exit keyed by
  the source position on both sides, and the first function whose exit differs is the one to
  read.  One function against one function is one debug round.

**Why.**  A re-derived elaborator agrees with Rust only in the final image.  When it does not,
the difference has no address: it takes several debug rounds to find the function, and an
effect of Rust's control flow (an unused `_elm_N`, a re-parse) is copied by a special case at
each site that shows it.  Porting Rust's own rule removes those special cases (the work-ref
step-over: `retypes_argument` and `argument_of_another_site` replaced three materialise-site
records).

**Rust's intricacy is the specification.**  The port does not start from a cleaner design and
add Rust's special cases back as gate E finds them; that is the rewrite trap, where every case
costs a debug round.  It copies them with the function that holds them.  Nor is Rust
simplified first to make the port easier: the Rust front end changes only to fix a defect.

**What gets transcribed.**  Every function a family still lacks.  Code that already agrees is
left as it is: a re-derived function is replaced by its transcription only when a difference
traces back to it.

## Syntax decisions that read semantics — each becomes a general node

The Rust parser takes a few SYNTACTIC decisions from semantic facts.  A survey of
`src/parser/` (2026-11-01, every `link()`/`revert()` and every `def_type`/`vars` read on a
syntax path) found each can be deferred: the syntax layer parses a node general enough to hold
every reading, and elaboration picks the reading with the fact the Rust parser read.

| Construct | Rust reads | Syntax node |
|---|---|---|
| `Name { … }` struct literal or a block after an expression (`objects.rs:2930`, `3102`) | `def_type(Name)`, `vars.name_exists` | `Path { body }` when the body is `ident:` / `"str":` / empty, never in a control head (`in_control_head`) |
| `{ f: v }` with no name (`control.rs:977`) | the target's type | `AnonRecord`, told from a typed local `{ x: T = … }` by `ident : Type =` |
| `Sh.Dot { r: 1 }` (`fields.rs:23`) | the variant's fields | a member followed by a brace body |
| `name<T>(…)` (`objects.rs:828`) | not a variable, not a type | call with type arguments |
| `Name<…>` in a type (`definitions.rs:4823`, `4886`) | `TypeTemplate` | type with arguments |
| `sizeof(…)`, `type_name(…)` argument (`control.rs:23103`) | `paren_type_ahead` | `TypeOrExpr` |
| `a.b(…)` method or a `fn` field called (`fields.rs:754`) | attribute `b` is a `Routine` | `MemberCall` |
| statement end after a block-like expression (`operators.rs:2108`, `2498`) | the left operand is `Void` | the block-like-statement rule: `if`/`for`/`while`/`match`/block end the statement |
| `x#index` (`objects.rs:992`) | `vars.name_exists` | postfix `#ident` |
| `v is V { a, b }` captures (`control.rs:14140`) | the variant has fields | a capture list when another `{` follows its `}` |
| a pattern identifier: binding, constant or variant (`control.rs:6555`, `11616`, `12701`) | the subject's type | one pattern grammar: `ident`, `ident : pat`, `Path ({…})?`, literal, range, group `* + ?`, `..rest`, `|` |
| `x[…]` index forms (`fields.rs:1613`) | the receiver's type | one index node `[args (.. =? expr?)? (: expr)?]` |
| format spec `{x:w}` (`objects.rs:4140`, `3603`) | the width names a variable; the type owns `to_text` | the spec's tokens kept as written, interpreted in elaboration |

## The token cursor and the arena

```loft
pub struct Syntax {
  src: vector<Tok>,          // the cursor's source: the first vector field
  pos: integer,              // the cursor
  farthest: integer,         // (P-Cursor) PC5's high-water mark — where a parse error points
  nodes: vector<Syn>,        // the arena; a node's children are indices into it
}
```

A rule answers the index of the node it built, or null when it does not match.  Children are
held as arena indices, never as `&v[i]` into a vector that grows (README § Sub-arcs, loft#2163).
Token positions stay in `Toks`; a node records the index of its first token, which names its
line and column.

## What the language needed — fixed in the language

A production is a sequence of tokens and rules, so the PEG cursor match was extended before the
grammar was written (`formal/matching.md` `(P-Cursor)`, `(P-Rule)`, `(WF-Term)`, `(WF-Pure)`;
cells under the suite's `rules/matching/P-Cursor` and `P-Rule`):

1. A sub-rule anywhere in a pattern: `[LParen, e: expr, RParen]`, read in segments, the rule
   called only when its arm is tried, the cursor put back when the arm fails.
2. A sub-rule group: `(args: expr)*(Comma)`, `+`, `(r: ret)?`.
3. Recursion through a consumed token is admitted; only a cycle of calls that can run before a
   token is consumed is left recursion.
4. A rule answers any type, so the arena form (`-> integer?`) is a rule.
5. A cursor match's mode covers its patterns, not its arm bodies (loft#2322).

A look-ahead is written as a PREDICATE rule — a function that answers the position without
consuming, or null (`brace_next`, `no_else`, `empty_literal_allowed`).

## Open

- **Lift after #2439:** `el_vars::take_over` fills `viewed` with a loop; its best form is the
  comprehension `[for _ in v.vars { [] }]`, refused today.
- **Diagnostics** are part of the port (same text, same position), measured after gate E on the
  programs `tests/parse_errors.rs` and the suite's refusal cells carry.
- **Scripts** (`src/script.rs`, a textual desugar before parsing) are not ported yet: gate S
  counts the file the driver would rewrite as a failure.

## Gates, in order

| Gate | Measures | Instrument |
|---|---|---|
| **T** tokens | the C lexer's stream equals `src/lexer.rs`'s, per file | `LOFT_TRACE_TOKENS=<file>` (the Rust stream the parser scanned) against `selfhost/front/tokens.loft`; `selfhost/front/tokens_gate.py` counts files that agree |
| **S** syntax | every corpus file parses with no error; the tree's tokens, in order, are the file's tokens | the syntax tree printer, round-tripped against the token stream |
| **E** elaboration | the image equals `--emit-image`: each definition, and the program's part of the schema | `selfhost/compare/`; HOW MANY programs agree |
| **E** contract | each function of an agreeing program carries the same behaviour contract (§ The behaviour contract) | `LOFT_REPORT_CONTRACT=1` on both front ends; `elab_gate.py`'s `contract:` line and the verdicts file's `contract` column; `elab_probe.py one` lists each fact |
| **E** state | each function of an agreeing program carries the same image state: the flags the later passes read, each variable's deps, the inline refs | `LOFT_REPORT_STATE=1`; `elab_gate.py`'s `state:` line (by kind), the verdicts file's `state` column, `LOFT_ELAB_STATE_FILE=<file>` for each differing item |
| **E** reasons | each image flag both front ends set on a variable is set by the same Rust rule | `LOFT_TRACE_VARFLAGS=*` on both front ends; `elab_gate.py`'s `reasons:` line (by flag), the verdicts file's `reasons` column, `LOFT_ELAB_REASON_FILE=<file>` for each differing flag |

The corpus for every gate: `default/*.loft`, `tests/scripts/`, `tests/docs/`, `lib/`, and the
language suite (`../loft-suite-91`).

Before an elaborator build, `selfhost/front/elab_pregate.py` runs `loft check` on
`selfhost/front/elab.loft` (front end only, a couple of minutes against a build-and-gate cycle
of about half an hour): any error fails it, and a warning or advice fails it only where it points
at a line `git diff` marks as changed — the lint of the edit.  The whole elaborator is checked:
a module checked as its own entry types its `use` graph in another order and reports errors
the real build does not.

The port has three purposes: the main implementation — a front end on C with no rustc to
bootstrap; a second implementation that VALIDATES the Rust one, finding loft's internal
problems; and a front end whose internal conclusions are visible early and easily — the
behaviour contract and state reports (§ The behaviour contract), which make these algorithms
debuggable and are a deliverable of the rewrite, not a by-product.  So a gate E difference is
triaged, never just copied:
- the port is incomplete → port what Rust does (1b-interpreter.md rule 5);
- Rust answers wrong (a silent wrong value, a refusal the formal rules say must work, a
  backend that does not build) → fix Rust, with a falsified suite cell;
- Rust carries an accident with no meaning (a variable minted and never used, an order that
  falls out of the pass structure) → log it under § Rust accidents and reproduce it only
  while gate E compares images byte for byte.

### The behaviour contract

The gate compares with STRICT equality (owner): a flag no Rust reader sees is compared all the
same — "that leads to far easier comparison of code.  If you dislike the rust implementation
that one has to move first."  An inert difference is removed by changing RUST first (with its
IR unchanged and its suite cells green on both backends), never by filtering the comparison.

The port follows Rust's REASONING, not its in-memory data structures: unless Rust is clearly
wrong, a conclusion is reached by the rule Rust reaches it by, because a port that gets the same
answer another way diverges where the corpus does not look.  Rust's rules carry months of
edge-case debugging; a port rule reached another way has to repeat it.  Rust's parser fields and side
tables are not compared one by one.  What is compared is the reimplementation contract below,
and a difference in it is triaged as a gate E difference:

- **the code** — the image's definitions and schema (gate E itself);
- **the image's function state** — per function its purity, what its result promises
  (`returned_not_null`, `returned_const`), the 32-bit regime, its closure's captures; per
  parameter a `const` read at every call and a default; per variable the flags the scope pass
  and the emitters read (`skip_free`, `deferred_first_bind`, the witnesses, `inline_refs`, …):
  `LOFT_REPORT_STATE=1` (`report::state_report`, printed by `defs` for Rust's image and by
  `elab` for its own);
- **the facts a caller acts on** — computed, not stored, so a mismatch shows only where some
  caller in the corpus exercises it: the buffer the result is delivered through, a bufferless
  text answer, a result that views a parameter, a fn-ref answered, a result the caller adopts
  fresh, a store-text instance, one of several arguments answered, a store forwarded that the
  function does not own, whether every exit fills the buffer, the operator a one-op wrapper
  forwards to, the parameters rebound with a store the body owns.  `LOFT_REPORT_CONTRACT=1`
  prints one `[contract]` line per program function: `src/parser/contract.rs` asks each fact
  of the predicate Rust's own call sites ask; `selfhost/src/el_contract.loft` asks this port's,
  and prints `?` for a fact it does not derive yet — a gap the gate counts apart, never a
  value guessed.
- **the rule behind each image flag** — which Rust function set a variable's `skip_free`,
  `value_const`, `inline_ref`, `captured`, `user_named`, `user_appended` or `copy_bound`.
  `LOFT_TRACE_VARFLAGS=*` prints a `[varflag]` line at each set: Rust gives its setter call's
  `file:line` (`trace_flag`, every setter `#[track_caller]`), which `elab_gate.py` resolves to
  the enclosing function; the port's setters (`el_vars::set_skip_free`, …) take the name of the
  Rust rule the site mirrors and print it in that place.

The reports are permanent features of both front ends, not gate scaffolding: they show a
function's conclusions early, which is what bug hunting in these algorithms needs (home doc:
`DEBUG_TRACE.md` § A function's contract and state).  The contract does not grow further unless a
concrete mismatch shows that a hidden fact is needed; then it goes where it fits (a
`[contract]` fact on both front ends, or a `[state]` line).  Small differences from Rust may
remain; a port rule that decides by a different reasoning than Rust's is a defect.  Decision traces (`LOFT_TRACE_RETPROMO`,
`LOFT_STATE_TRACE`) explain why an output differs; they are no part of the contract.

Rules of decision ORDER — what a later decision sees is what Rust had at that moment:

- `@FR-F-ParamTuple`'s shadow is decided against its copy seed: on pass 2 `parse_function`
  builds the seed (`tuple_whole_bind_copy`, minting its `__ref_p2_N`) and gives the shadow the
  COPY's type before the body is read (definitions.rs ~3699–3705); only the finish
  (`tuple_param_seed`) retypes it to the view where `MemberWalk::only_read` holds.  So the body
  is parsed with `__tp_p` borrowing `__ref_p2_1`: `fn pick(p: (R, integer)) -> R { if p.1 > 0 {
  p.0 } else { R { … } } }` reads its tail as viewing a local (`return_views_local`), takes
  `MaterializeView`, and mints and unregisters `__ref_1` (traced by the tester with
  `LOFT_LOG=type_timeline:__tp_p`; listed as a Rust accident before that).

### Rust suspects

Rust rules whose reasoning does not hold together, seen while porting them or where the
varflag trace sets two rules side by side.  For each, try to write loft programs that are
actually wrong because of it (a silent wrong value, a leak, a double free, a lint wrongly fired
or silent) on both backends; a wrong program found is filed as an issue (ISSUE_TRACKING.md) and
fixed in Rust with it as the suite cell, and the port then follows the fix.  A suspect no wrong
program was found for stays listed with what was tried.

- `parse_var_inner`'s mention mark: a parameter is mentioned in a field append (`s.h += …`) and
  a tuple member write (`t.0 += …`) but not as a tuple argument read (`pick(p)`) — the two front
  ends split these differently (13 flags over the corpus).  Tried: every `_`-named user
  variable in a tuple, tuple-member write, field append, loop, vector and text copy, match and
  return ends user-named and answers as its plain-named twin (`is_compiler_generated` reads
  the name), so the split is in which path marks, never in the flag; no wrong program found.
- `collect_mutated_captures` takes its candidate names from the lambda's whole variable table
  (it runs before the closure record exists), so a capturing lambda's `mutated_captures` lists
  its own written locals and temporaries (`i`, `i#index`, `_range_end_1`, `__ncc_1`) beside
  the captures it writes — its comment calls the over-inclusion "safe (just spurious work)".
  Tried: an outer scalar and an outer vector parameter named as the lambda's own loop variable
  (`for i` inside, `i` outside), with and without a capture — the readers intersect with the
  real captures or read "written" conservatively: right answers on both backends, no
  diagnostic; no wrong program found.  The port asks per capture (`writes_a_name`) and so
  differs on `mutated_capture` in 21 programs: porting Rust's rule is the IR walk
  `walk_for_mutations` over the variable table.
- `unbox_tuple_from_dbref` marks its temp `skip_free` and leaves it a work ref, where the
  parallel paths unregister theirs (`unregister_work_ref`).  Not observable: the image is
  identical either way (gate E agrees on the 7 functions it occurs in).

### Rust accidents

- A capture read on pass 1 makes a placeholder variable of the capture's name in the lambda's
  table (`create_var` + `mark_user_named`, objects.rs `parse_var_inner`'s capture arm), and the
  table keeps it into pass 2, where the read goes through the closure record instead.  The port
  makes no placeholder: the `user_named` difference on capture names (`e`, `s`, `v` … in
  lambdas, about 60 items).  Probed: `stack_trace()` inside a capturing lambda lists its
  parameters and locals only — the placeholder is not live in the frame; no wrong program.
- A call answering a record `τ?` that borrows a parameter (`fn whole(s: S) -> S? { s }`) is
  typed borrowing a VARIABLE argument but not a literal one: `whole(S { a: 7 })` is `ref(S)?`
  with no dep on the literal's store, where the dense `-> S` names it.  Reproduced; probed
  (both backends, `LOFT_POISON` + `LOFT_STRICT_STORES`, the result outliving the literal's
  block, churn between): no fault — the record lives in the caller's return buffer.

- A record literal with a vector-literal field written into a place (`xs[i].m = V { a: [1] }`)
  is built apart and copied in; the first pass's in-place attempt leaves an `_elm_N` the second
  never uses, still declared at the function head.  The port refuses this shape by name.
- A literal of a droppable rebinding a local (`s = RbS { h: mk(…) }`) is tried in place, then
  parsed again apart; the first attempt's work refs stay declared, unused (`abandon_work_refs`
  reproduces that).  Where a field value reads the local (`h: mk(s.h.id)`), the attempt also
  stages it, one more `__ref_N` the port does not mint: refused by name.

## Where it stands

| Gate | Measured | Remaining |
|---|---|---|
| T tokens | 1845 of 1850 files | four `"\0"` literals (a one-NUL text is loft's null text), one script |
| S syntax | 1836 of 1843 files | seven refusal cells, all `@EXPECT_ERROR` files |
| E image | 718 of 1825 programs agree, 1 differs (`selfhost/front/elab_gate.py`, at `6ca9cf280`); 250 more are programs the Rust front end itself refuses (an `@ARGS: --lib` is passed to both) | the one difference (C-Var/2441: the compiled elaborator frees a store twice), then the families `elab_gate.py --plan` ranks first |
| E by rule | 60 of 379 formal rules carried for every suite cell, 94 for some, 225 for none; 254 of 1073 suite cells (14 rules have only cells the Rust front end refuses) | dispatch (5 of 28 cells: free overload sets carried; methods in a set, generic members, run-time variant dispatch refused by name), tuples (0 of 22), closures (1 of 47), calls (6 of 50), interfaces (2 of 38) |

**Measuring by formal rule.**  The suite holds its cells under `rules/<chapter>/<rule>/`, so a
gate run's verdicts file reads per rule: a rule is carried when every one of its cells agrees.
It measures the front end only, and pessimistically — a cell crosses several rules, so a
refusal from another chapter blocks a cell whose own rule is carried (dispatch blocks most
interface and call cells).  The empty chapters are each ONE unported family: calls not by a
bare name (dispatch), tuple members, lambdas that capture a local.

```bash
awk -F'\t' '$2 ~ /^loft-suite-91\/rules\// { split($2, p, "/"); r = p[3] "/" p[4]; n[r]++;
  if ($1 == "agree") a[r]++; if ($1 == "noimage") x[r]++ }
  END { for (r in n) { m = n[r] - x[r]; k = a[r] + 0;
    print (m == 0 ? "n/a" : k == m ? "full" : k == 0 ? "none" : "part") "\t" k "/" m "\t" r } }' \
  ~/workspace/.pln91-gate/verdicts-<sha>.txt | sort -k3
```

Elaboration is `selfhost/src/el*.loft`; gate E's runner is `selfhost/front/elab.loft` (`elab
<image> <program>` over a `LOFT_EMIT_PARSED` image, or `elab <stdlib files>`), its oracle side
`selfhost/compare/defs.loft`.  A construct elaboration does not carry yet is refused BY NAME
(`refused: …`), never approximated.

**Measuring a change against gate E.**  The count alone hides a swap (one program gained, one
lost).  Run the gate with `LOFT_ELAB_VERDICTS=<file>` on the change and on its base — the base
from a `git worktree add <scratch>/head HEAD`, built there — and diff the two files: every line
that moved is a program to read.  Run it with `--jobs 6` under `ulimit -v 6000000`: the default runs
one `elab` per core, and some corpus program drives one to 11 GB, which on a 60 GB box brought
in the kernel's OOM killer and took the whole session with it.  `--plan` ranks the refusal families by how many programs each
alone blocks; `--which TEXT` lists the programs that meet one; `LOFT_ELAB_KEEP=1` keeps the
build directory for a single-program diff.

**When the elaborator's own output goes wrong** — a vector that empties, a C refusal, a helper
answering null — suspect the LANGUAGE first: reduce it to a minimal probe, run it on both
backends and on installed main.  Most such failures in this port were language defects
(loft#2337–#2346), each fixed on the branch with a suite cell rather than spelled around.
Record with each such defect the rules its reduced repro crosses — the `@FR-` tag of every rule
it meets, and any axis no rule names (pass order, a callee declared later) — in the issue, or in
the commit for one fixed at once.  A three-rule defect is the class the rule-pair walk cannot
reach, and that record is how this front end's share of them is counted.

The syntax layer is `selfhost/src/syntax.loft` (the rules, one per production of
[1a-grammar.md](1a-grammar.md)); gate S's runner is `selfhost/front/parse.loft`.

## Order of work

1. Elaboration, pass by pass, in the Rust parser's order — definitions first, then the
   bodies; gate E per definition so a partial port is measured.
2. The transcription, family by family ([§ The porting method](#the-porting-method--transcribe-do-not-re-derive)):
   dispatch first (`src/parser/dispatch.rs` → `selfhost/src/el_dispatch.loft`, gated by the
   suite's `rules/dispatch/` cells), then calls, closures, tuples and interfaces.  The
   function-level state trace comes with the first family.
3. The script desugar, then diagnostics.
