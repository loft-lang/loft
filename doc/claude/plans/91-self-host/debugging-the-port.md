# Debugging the loft front end

How to see what the loft-written elaborator (`selfhost/src/el*.loft`) did to a program, and how
it differs from the Rust front end.  One switch, `LOFT_STATE_TRACE=1`; one reader,
`selfhost/front/elab_trace.py`.  The design and what each piece measured are in
[elaborator-instruments.md](elaborator-instruments.md) and
[port-debug-output.md](port-debug-output.md).

## Start from the question

| you want to know | run |
|---|---|
| which statement created a variable, or changed its type | `elab_trace.py PROG --var NAME` |
| everything one function did on the first pass | `elab_trace.py PROG --fn NAME --pass 1` |
| where the elaborator gave up, and which special paths it took | `elab_trace.py PROG --points refuse,route` |
| where it differs from the Rust front end | `elab_trace.py PROG --side diff` (add `--fn`, `--pass`) |
| the IR it wrote for one function | `elab_trace.py PROG --ir NAME` |
| … against the Rust front end's IR | `elab_trace.py PROG --ir NAME --side diff` |
| the same over the whole corpus | `elab_gate.py --pass1` (below) |

`--fn main` finds `n_main`; a method is `t_<LEN><Type>_<name>`; a lambda is
`n___lambda_<function>_<n>`.  The reader builds the elaborator once into `target/elab-trace/`
and again when `selfhost/` changes (minutes); `--elab`, `--defs` and `--std` take built ones.

## Reading a line

```
state <pass> <line>:<col> <point> …
```

`<pass>` is 1 or 2.  The position is where the front end's lexer stands with the named token as
its peek — the END of that token, so a statement starting at column 3 with a two-letter name
reads `:5`.

| point | printed | says |
|---|---|---|
| `stmt` | before a statement is read | what it was handed: `expected=<shape>`, `assign_target=<nr>(<name>)`, `in_bind_rhs`, `cur_type_vars=` — each only when set |
| `arg` | before a call's argument is read | the same channels, for the argument |
| `stmt-end` | after the statement | `value=<type>`: what its expression answered |
| `var` | after `stmt-end` | what the statement did to the variable table: `<fn> <nr> <name> + <type>` created, `<fn> <nr> <name> <old> -> <new>` retyped |
| `fn-end` | when the function's body is done | `<fn> ret=<type>` (with `attrs=<n>:<attribute name>,…` where the result has deps: they number the definition's ATTRIBUTES, while the type spells the variable of that number — `["__retbuf"] attrs=1:cv` is a buffer renamed after `cv`, `attrs=1:__retbuf` one that was bound), then `<fn> var <nr> <name>=<type>` for every variable in number order |
| `route` | where a special path is taken | the Rust handler's name (`parse_map`, `materialize_return_into=whole`); the port prints the same name at its mirror |
| `refuse` | where the port gives up | the refusal's text (the port only) |

Types on `stmt-end`, `var` and `fn-end` are spelled as the IR dump spells them, deps by
variable name: `vector<text["xs"]>["__vdb_4"]`.  `stmt` and `arg` use a short shape without
deps (`vector<text>`, `ref(Node)`).

Three things to know when reading:

- **A change belongs to the innermost statement that ended after it.**  A block or a lambda
  body prints its own statements' `var` lines first; the statement around it prints what is
  left.
- **The first statement's `var` lines include the parameters** and the buffers made before the
  body: the table as the body starts.
- **A variable created after the last statement** (by the return path, when the body ends) has
  no `var` line; `fn-end` lists it.  Compare the last `var` number with `fn-end`'s.

## Against the Rust front end

Both front ends print the same points.  `--side diff` shows the first pass and the second; the
usual order of reading a difference:

1. `fn-end` differs — which variable, or the result.
2. `--var NAME` on both sides (`--side both`) — which statement created or retyped it
   differently.
3. The `route` lines at that statement (`--points route,refuse`) — which Rust code handled it,
   and whether the port took its mirror.

`stmt` and `arg` often differ without a consequence: Rust keeps `expected` as one field that
lingers between sites, the port carries the hint per node.  The gate does not count them.

## Over the corpus

`python3 selfhost/front/elab_gate.py --pass1` adds to gate E:

- the first-pass verdict by function (`pass1-agree` / `pass1-differ`), the `elab-pass1`
  baseline block and `PASS1 DROP` for a pinned function that stopped agreeing;
- `LOFT_ELAB_PASS1=<file>`: per compared function the first differing line of each side and
  the statement where the variable tables parted;
- `LOFT_ELAB_PASS1_REFUSED=<file>`: the same for the functions a refusal is met in — a refused
  function whose first pass agrees needs a second-pass lowering only;
- `LOFT_ELAB_ROUTES=<file>` and `LOFT_ELAB_ROUTE_COVERAGE=<file>`: every `route` / `refuse`
  line, and per Rust handler where the port takes its mirror, refuses, or does neither;
- a positive control and a self-check on every run: the layer must be able to fail, and
  neither side's `stmt`, `stmt-end`, `var`, `fn-end` may be missing.

## Before promoting: the try-gate

`scripts/try_gate.sh` HANDS the work in a checkout — committed or not — to the testing side
before it is promoted, and returns at once: the change goes as a `git bundle` in a comment on
the mailbox issue (loft#2560, closed and locked on purpose).  The testing side gates it.
A CLEAN result — no differ, no program and no definition that agreed and no longer does, no
first-pass drop — is acknowledged automatically as a comment `result <commit>`: safe to
promote.  One with ISSUES gets the same automatic line ("do not promote this one yet"), and the
tester's analysis follows as a message: each definition that stopped agreeing with the refusal
and source line that caught it, each differ with the first differing line of both sides.
Neither is waited for: post, go on to the next item, read the answer at a natural break.
Either answer lists what the change MOVED — the programs that newly agree, and for each program
that lost a refusal and is still refused, the refusals left — so a listing of the refusals over
the corpus before promoting is not needed.  A lifted program that DIFFERS (a neighbouring shape
the refusal had hidden) comes back analysed: what Rust decides there, and whether it is carried
or wants a refusal by name.  Nothing is pushed to a branch.  Use it for a change that adds or
widens a refusal, or changes a rule many functions share: a definition that agreed inside a
program that stays refused is seen by nothing else before the promotion.

## What the trace does not show

- Nothing below a statement: an operator's or a sub-expression's type is in the IR
  (`--ir NAME`), on the second pass only.
- No column on a refusal met in a DECLARATION (a struct field, a signature): `:0`, the line the
  declaration's.
- The Rust front end's other instruments (`LOFT_LOG=type_timeline:<var>`, `LOFT_VAR_TABLE`,
  the `LOFT_TRACE_*` family) have no twin in the port; `var` and `fn-end` answer what the first
  two answer, in a form both sides print.
- The loft front end is not the `loft` command's yet, so there is no command-line flag: the
  reader is the entry.
