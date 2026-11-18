# Debugging output of the loft front end (@PLN91)

Design by the tester on the owner's request (2026-11-18): the loft-written elaborator must
give solid debugging output of its own, because the benefit of the instruments is future bug
fixes — made in the loft code once the Rust front end is gone.  It extends
[elaborator-instruments.md](elaborator-instruments.md): the same switch, the same line form,
the same rule that nothing changes with the switch off.  One commit per item; each item's
"Built" note says what was built and measured.

## The questions a bug fix asks

| question | answered by |
|---|---|
| which statement created this variable, on which pass? | § 1 `var … +` |
| which statement changed its type or its deps, from what to what? | § 1 `var … ->` |
| what did the pass leave for the function? | `fn-end` (built) |
| what did this statement answer? | `stmt-end` (built) |
| where exactly did the port give up, or take a special path? | § 2 the column |
| what is the IR of this one function? | § 3 `--ir` |
| all of it for one program, one function, one variable, without setup? | § 3 the runner |
| is an instrument still printing? | § 4 the self-check |

## 1. A variable's history — the `var` point

Both front ends print, after each `stmt-end`, what the statement did to the function's
variable table, in number order:

```
state <pass> <line>:<col> var <fn> <nr> <name> + <type>          created by this statement
state <pass> <line>:<col> var <fn> <nr> <name> <old> -> <new>    its type or deps changed
```

The position is the statement's (its `stmt` line's), the type spelling `fn-end`'s.

It is computed by DIFFERENCE, not by a hook: each side keeps, per function and per pass, the
spelled types it last printed, and at `stmt-end` compares the table with them.  Three reasons:

- the Rust table is written from `src/variables/`, which knows no pass and no position; the
  port's is written through `el_vars::add_variable`, `create_unique`, `set_type` and eight
  direct field writes.  A hook at each is many sites to keep alike on two sides; the
  difference is ONE site on each and cannot miss a write;
- a statement nested in another (a block, a lambda's body) prints its own changes first, so
  the outer statement prints only what is left: every change is attributed to the innermost
  statement that finished after it;
- parameters and the buffers minted before the first statement appear at the first
  statement's `var` lines, marked as created — the table's state where the body starts.

A lambda is a function of its own, with its own table.

**In the gate.**  `var` lines are NOT part of the `--pass1` verdict: that layer compares what a
pass left, and a table that differs in between and agrees at `fn-end` is not a difference of
state handed on.  They LOCATE: for a function that is `pass1-differ` at `fn-end … var`, the
first differing `var` line names the statement where the two tables parted.  `pass1.tsv` gains
that line as two more columns, and `pass1_mint.py` prints it.

### Built — § 1

`Parser::state_trace_vars` and `el_trace::vars`, each called after its side's `stmt-end`; the
printed table is kept per function and pass (`Parser::trace_vars`, `El.trace_vars`).  The gate
leaves `var` lines out of the verdict and writes, for each `pass1-differ` function, the first
`var` line the two sides do not share as two more columns of `LOFT_ELAB_PASS1`'s file; the
tester's `pass1_mint.py` prints it as "parted at".

Whole corpus (loft30-gate 7ef888315): the verdicts and the first-pass counts are unchanged
(1131 of 2085; 10748 of 15932 functions).  Of the 5184 functions that differ on the first
pass, 3580 have a statement where the variable tables parted; the others differ in a
statement's answered type only, or in a variable created after the last statement, which only
`fn-end` shows (`r1`'s `__ref_3` in the port, before the porting agent's f74d7e42b).

## 2. Where exactly — the column on the port's `route` and `refuse`

The port prints `route` and `refuse` at `<line>:0`: `el::refuse` knows the line of the last
statement marker only.  The elaborator gets ONE field, `El.at` — the first token of the
expression node being elaborated — set and restored by `el_ex_assign::expression`, the one
function every expression node goes through.  `el::refuse` and `el_trace::route` print that
token's line and column; outside any expression (a declaration) they keep today's line with
column 0.

No refusal site changes: the 655 sites keep calling `el::refuse(e, text)`.  The position is
the innermost expression the port was elaborating when it refused, which is the construct the
refusal is about.

`refused-in:` (the gate's where-list) keeps the statement line it has; the trace line is the
exact one.  The join with Rust's `route` lines stays by line: Rust's column is its lexer's
position, which is where a construct's handler was ENTERED, not where the construct starts.

### Built — § 2

`El.at`, set and restored by `el_ex_assign::expression` (its body is `expression_node` now):
the node's leftmost token where `el_ex_null::node_start` knows the shape, the node's own token
otherwise.  `el::trace_pos` spells it for `el::refuse` and `el_trace::route`, in the
convention the other state lines use (`el::pos_at`, the Rust lexer's position while that token
is its peek).

Whole corpus (on f74d7e42b): verdicts and first-pass counts unchanged.  15558 of the 17246
`refuse` lines and 44837 of the 45005 port `route` lines carry a column; the 1688 refusals
without one are met outside any expression (a declaration).  The exact line also joins
better with Rust's routes than the statement's line did: `parse_match` 3342 of 3376 sites on
both sides (3313 before), `parse_vector_match` 822 of 854 (814), `parse_object` 8036 (7677);
3 of 30 routes are carried in every program that takes them (2 before).

## 3. One command — `selfhost/front/elab_trace.py`

```
elab_trace.py PROG [--fn NAME] [--var NAME] [--pass 1|2] [--points stmt,var,…]
              [--side port|rust|both|diff] [--ir NAME]
```

- builds the two tools once into `target/elab-trace/` and again only when `selfhost/` is newer
  (`--elab` / `--std` take built ones, as `state_diff.py` does);
- `--side port` (default) prints the port's trace; `rust` the Rust front end's; `both` side by
  side by position; `diff` is `state_diff.py`;
- `--fn` keeps the lines of one function (cut as the gate cuts them, at `fn-end`), `--var` the
  `var` and `fn-end … var` lines of one variable — its history over both passes;
- `--ir NAME` prints the definition's section of the port's report (the IR it wrote), with
  `--side both` beside the Rust front end's.

`state_diff.py` stays the comparison; this is the reader.

### Built — § 3

`selfhost/front/elab_trace.py` as designed, with `--defs` beside `--elab` and `--std` for the
report writer `--ir` asks of the Rust side.  `--fn main` finds `n_main`; a function's lines are
cut as the gate cuts them, a `var` line going to the function it names.  `--side diff` is a
unified diff of the two filtered traces, in which Rust's `route` lines and the port's stand
beside each other.  Tried on the probes of this plan: `--var p` prints the four lines of the
variable's history; `--ir r1 --side diff` reads "the two sides agree".

## 4. The self-check — an instrument that went silent

Every `--pass1` gate run already reads one agreeing program again for its positive control.
On that program the gate also requires that EACH side printed at least one line of each point
it is expected to print (`stmt`, `stmt-end`, `var`, `fn-end` on both; nothing is required of
`route` / `refuse`, which a program may not reach) and stops with the missing point's name
otherwise.  A Rust test (`tests/state_trace.rs`) pins the Rust side's lines for one small
program without building any tool.

### Built — § 4

The gate keeps the set of points each side printed per program (`SEEN_POINTS`) and, on the
positive control's program, stops with "an instrument went silent" naming the side and the
point when `stmt`, `stmt-end`, `var` or `fn-end` is missing from either.  Tried both ways: it
passes on a sample, and with a point no side prints added to the required list it stops with
"the Rust front end printed no `bogus` line for …".  `tests/state_trace.rs` pins the 27 lines
of one function's first pass on the Rust side — every point kind — and that nothing is
printed with the switch off; it runs with the `parser` subject.

## 5. The page

`debugging-the-port.md` beside this file: the questions above, the command for each, how to
read a line, and what the trace does not show.  DEBUG_TRACE.md points to it.

## What is deliberately not built

- No hook in `el_vars`'s writers (see § 1); routing the eight direct `type_def` writes through
  `set_type` would be a clean-up of the port's own, for the porting agent to judge.
- No per-refusal token: § 2 gives the innermost expression without touching a refusal site.
- No `loft` command-line flag: the loft elaborator is not the CLI's front end yet; the runner
  is a script until it is.
- No port of `LOFT_LOG=type_timeline` or `LOFT_VAR_TABLE` as such: § 1 and `fn-end` answer what
  they answer, in a form both front ends print alike.

## Order

1. § 1 `var` on both sides, the gate's two columns, `pass1_mint.py`.
2. § 2 the column.
3. § 3 the runner with `--ir`.
4. § 4 the self-check and the Rust test.
5. § 5 the page.
