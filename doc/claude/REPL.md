<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The loft REPL and introspection

This page explains two tools for trying loft and looking inside it:

- the **REPL** — an interactive prompt where you type loft one line at a time
  and see the result right away;
- the **introspection** output — bytecode, generated Rust, and variable slot
  tables for any program or REPL session.

Both ship with the `loft` binary. No setup is needed.

## Start the REPL

Run `loft` with no file, or `loft repl`:

```
$ loft
loft REPL — :help for commands, :quit to exit
loft> 1 + 2
3
loft> x = 40 + 2
loft> x
42
```

Type an expression and its value prints. Type `x = …` to name a value; the name
stays available on the next line. Press Ctrl-D (end of input) or type `:quit` to
leave. Arrow keys recall earlier lines (history is kept in `~/.loft_history`),
and Ctrl-C abandons the line you're typing. Press **Tab** to complete a name —
functions and types (yours and the stdlib's) and the variables you've bound —
or a `:command` when the line starts with `:`. After a `.`, Tab completes
**members**: a value's methods — shown with a trailing `(` so you can see
they're callable (`"hi".` → `starts_with(`, `length(`) — a struct variable's
fields (`p.` → `x`, `y`), and an enum type's variants (`Color.` → `Red`,
`Green`). (A member after an index or call result — `xs[0].` — needs type
inference the completer doesn't run yet, so it offers nothing rather than
guessing.)

### Your session is restored next time

Start the REPL interactively and it replays the bindings and definitions from
your last session, so `x`, your functions, and your structs are still there:

```
$ loft
restored 2 statement(s) from last session
loft> dbl(x)
84
```

The session is saved to `~/.loft_session` — only the state-changing lines, not
the expressions you merely printed. Start clean with `loft repl --fresh`, or
`:reset` mid-session to clear both the live state and the saved file. A saved
line that no longer parses (say, after a loft upgrade) is skipped on replay and
never blocks the rest.

Because resume re-runs your bindings, a value drawn from `random()` or `now()`
is produced afresh — the original stream is not reproduced (set an explicit seed
if you need reproducibility). Auto-resume is interactive-only: piped or scripted
input never reads or writes the session file, so captured output stays stable.

### Define things and call them

A function or struct you define stays defined for the rest of the session:

```
loft> fn dbl(n: integer) -> integer { n + n }
loft> dbl(x)
84
loft> struct Point { x: integer, y: integer }
loft> Point { x: 1, y: 2 }
{x:1,y:2}
```

A value prints in loft's own form: `42`, `{x:1,y:2}`, `[1, 2, 3]`. (This is not
JSON — struct fields have no quotes. Add the `:j` format to get JSON, e.g.
`"{p:j}"`.)

### Multi-line input

If a line is not finished — an open `{`, `(`, `[`, an unterminated `"…"`, or a
trailing operator — the prompt changes to `..... >` and waits for the rest:

```
loft> fn add(a: integer, b: integer) -> integer {
..... >   a + b
..... > }
loft> add(2, 3)
5
```

### Mistakes don't end the session

A typo prints an error and the prompt comes back; everything you defined before
is still there. A run-time error (such as a failed `assert`) is caught the same
way — the session keeps going.

## Finding what there is

Five commands answer "what can I use?" without leaving the REPL, and without a network:
they read the feature catalogue built into loft and the registry index already on your
machine (`loft search` or `loft install` fetches it once).

| Command | What it shows |
|---|---|
| `:features [word]` | Every language feature, grouped (values and types, collections, control flow, …). With a word, the features that mention it (`:features slice`). `:features inside` lists how loft itself is built. |
| `:libs` | Every library in the registry, grouped by category, with its newest version, whether it is installed here, and its description. |
| `:api <library> [word]` | Everything a library offers — each public function and type with its doc — or only the items whose signature contains `word`.  It shows the version this project's `loft.lock` pins, else the newest. |
| `:ops <Type>` | What a type can be written with: each operator and the definition behind it, its `[ ]` forms, and the interfaces it meets. |
| `:doc <name>` | One thing in full: a feature (`:doc @F2`, `:doc ??`, `:doc match`), a function, type or method (`:doc split`), a library function (`:doc time::from_ymd`), or a library (`:doc time`). `:doc <feature> run` runs the feature's example in a scratch session, so its definitions never meet yours. |

The same text is on the web pages: both are rendered from one source.

## REPL commands

Commands start with a colon:

| Command | What it does |
|---|---|
| `:help` (`:h`) | List the commands. |
| `:quit` (`:q`) | Leave the REPL. |
| `:reset` | Forget everything you defined; the standard library stays loaded. |
| `:fns` | List the functions you have defined, with their return type. |
| `:vars` | List the variables you have bound, each with its current value. |
| `:type <expr>` | Show the type of an expression without running it. |
| `:break <fn>` | Set a breakpoint at the start of a function's body — also `<fn>:<line>` (a line in that function), or `<fn> if <cond>` / `<fn>:<line> if <cond>` (a **conditional** breakpoint: break only when `<cond>` holds at the frame). Both are *function-scoped*, the only form unique in the REPL (every input restarts line numbering under the synthetic `<repl>` file, so a bare line isn't unique; file:line is for a file-run debugger). `:break` lists, `:break clear` removes. The next call that reaches it **suspends** into the paused sub-mode (below). |
| `:trace <fn> <expr>,…` | Set a **tracepoint**: on each hit, log the comma-separated expressions and **continue** (never pauses). A non-interactive log of values at a point. |
| `:bytecode [fn]` | Show the bytecode — all your functions, or just the named one. |
| `:rust [fn]` | Show the Rust code loft generates for native compilation. |
| `:slots [fn]` | Show each function's variable slot table (name, type, slot range). |

```
loft> fn dbl(n: integer) -> integer { n + n }
loft> :fns
dbl -> integer
loft> :slots dbl
fn n_dbl:
  #    arg  name   type   scope  slot         live
  ----------------------------------------------------------------------
  0    arg  n      int    0      [0, 8)       -
```

### Conditional breakpoints and tracepoints

A plain breakpoint stops on *every* call. When a bug only shows up on *one* call —
the classic "fails once in 10 000" — add a **condition**: the run only suspends when
the predicate over the frame is true, silently running past the rest.

```loft
loft> :break update if entity.health < 0
breakpoint set: update if entity.health < 0
loft> game_loop()
⏸ paused in update | entity = Entity{health:-3,…}
```

A **tracepoint** is the opposite of a stop: on each hit it logs some expressions and
keeps running, giving you a value trace without pausing — the non-interactive way to
watch state evolve over a whole run.

```loft
loft> :trace move { x, y }
tracepoint set: move { x, y }
loft> game_loop()
⤳ trace | x = 4, y = 7, x = 5, y = 7, x = 6, y = 7
42
```

**When to use which:** a *condition* when you know the bad state but not which call
produces it (break there and inspect); a *tracepoint* when you want to see how a value
changes across many calls without stepping. Both reach for the same frame-evaluation as
the `(dbg)` prompt — they're "the breakpoint, but it decides whether to stop."

### Paused at a breakpoint

When a call reaches a breakpoint the REPL **suspends** inside that frame and the
prompt changes to `(dbg)`. The frame's in-scope variables are shown, and the frame
*is* a REPL: you inspect its variables, change them, step through the code, and undo
a change you didn't mean. The verbs work with or without the leading colon (`step` or
`:step`).

```loft
loft> :break calc
breakpoint set: calc
loft> calc(5)
⏸ paused in calc | n = 5
(dbg) n * 3                  # read: evaluate against the live frame
15
(dbg) n = 99                # edit: write straight into the frame
⏸ paused in calc | n = 99
(dbg) :undo                 # changed your mind — revert the edit
⏸ paused in calc | n = 5
(dbg) :continue             # resume; the rest of calc runs with n = 5
▶ resumed — run finished
50
```

Quick reference for the `(dbg)` prompt:

| Input | What it does | Reach for it when |
|---|---|---|
| `:step` (`:s`) | Run to the next source line, **into** any call. | You want to descend into a function the current line calls. |
| `:next` (`:n`) | Run the current line's calls to completion, **over** them, then stop at the next line. | You trust the called functions and only care about *this* function's flow. |
| `:finish` (`:o`) | Run to the current function's return — **out** to the caller. | You've seen enough of this frame and want to pop back up. |
| `:continue` (`:c`) | Run to the next breakpoint, or to the end of the call. | You're done stepping and want the program to run on. |
| `:vars` | Re-show the current frame's variables. | After a few steps or edits, to re-orient. |
| *any expression* | **Evaluate** it against the frame's live variables and print the value. | To probe state — `pt.x * pt.y`, `items.len()`, a predicate. |
| `name = <expr>` | **Edit** a local (see *Editing live values*). | To try a "what if" without changing the source and re-running. |
| `:undo` (`:u`) / `:redo` (`:r`) | Step **back** / **forward** through this suspension's edits. | You over-edited, or want to compare before/after on resume. |
| `:quit` (`:q`) | Leave the REPL. | |

#### Stepping through the code

`:step` descends **into** calls; `:next` runs them to completion and steps **over**;
`:finish` runs **out** to the caller; `:continue` runs on to the next breakpoint or
the end. Stepping moves by **source line**, and any body line is a valid stop.

```loft
loft> :break outer:2          # break on line 2 of `outer`
loft> outer(5)
⏸ paused in outer | n = 5
(dbg) :step                   # into the call on this line
⏸ paused in inner | x = 5
(dbg) :finish                 # back out to outer
⏸ paused in outer | n = 5, a = 6
(dbg) :continue
▶ resumed — run finished
106
```

**When to use which:** reach for `:step` to investigate a suspect callee, `:next`
to stay at the current level and watch locals evolve line by line, `:finish` once a
frame has told you what you need, and `:continue` to jump to the next breakpoint (set
a *conditional* breakpoint first if you only care about one specific iteration).

#### Inspecting the frame

Type any expression and it is evaluated against the paused variables — every type,
just like the top-level prompt but scoped to the function you're stopped in. `:vars`
re-prints the whole frame.

```loft
⏸ paused in area | pt = Point{x:3,y:4}, k = 2
(dbg) pt.x * pt.y           # struct fields
12
(dbg) pt.x + k              # mix locals
5
(dbg) :vars
⏸ paused in area | pt = Point{x:3,y:4}, k = 2
```

**When to use it:** to confirm an assumption about live state before you step or
edit — read a field, call a method, test a boolean — without touching the program.

#### Editing live values

`name = <expr>` writes straight into the live frame, and the resumed call uses the
new value — a no-rebuild "what if". The RHS is evaluated against the frame, so
`n = n + 1` and `b = !b` work. You can edit:

- a **scalar** local (`integer` / `float` / `single` / `boolean` / `character`),
  `text`, or a simple enum — `n = 99`, `msg = "retry"`, `state = State.Done`;
- a scalar **struct field**, including nested inline paths — `pt.x = 9`,
  `pt.inner.x = 0`;
- a scalar **vector element** — `v[1] = 42`;
- a **whole heap value** — replace the entire struct, vector, or struct-enum:
  `pt = Point { x: 10, y: 20 }`, `v = [40, 50, 60]`.

```loft
⏸ paused in greet | pt = Point{x:3,y:4}, msg = "hi"
(dbg) pt.x = 9                       # one scalar field
⏸ paused in greet | pt = Point{x:9,y:4}, msg = "hi"
(dbg) pt = Point { x: 1, y: 1 }      # the whole struct, built fresh
⏸ paused in greet | pt = Point{x:1,y:1}, msg = "hi"
(dbg) :continue
▶ resumed — run finished
```

**When to use it:** to test a fix or reproduce a corner case in place — force a
boundary value, swap in a different struct, flip a flag — then `:continue` and watch
the consequence, all without editing and recompiling the source. **Not yet:**
replacing a *non-scalar* field or element in place (`pt.inner = Point{…}`,
`items[0] = Point{…}`) — rebuild the whole containing local instead.

#### Undo and redo

Every edit is recorded, so `:undo` reverts the last one and `:redo` re-applies it.
A fresh edit after an `:undo` **forks** the timeline (the redo history is dropped).
Undo/redo cover edits at the **current** pause point — resuming (`:step`/`:continue`)
starts a fresh history, because stepping reuses the frame's stack slots.

```loft
⏸ paused in calc | n = 5
(dbg) n = 7
(dbg) n = 8
(dbg) :undo                 # → 7
⏸ paused in calc | n = 7
(dbg) :undo                 # → 5 (the original)
⏸ paused in calc | n = 5
(dbg) :redo                 # → 7 again
⏸ paused in calc | n = 7
```

**When to use it:** when a "what if" edit didn't pan out, or to flip between the
original and the edited value before deciding which one to `:continue` with.

#### Watchpoints — break when a value changes

A breakpoint stops at a *place*; a **watchpoint** stops when a *value* changes,
wherever that happens. `:watch <expr>` watches a scalar struct field (`pt.x`) or
vector element (`v[i]`); on `:continue` (or any step) the run stops the moment a write
changes it, telling you the old and new value. `:watch` lists them, `:watch clear`
removes them.

```loft
⏸ paused in bump | c = Counter{n:5}
(dbg) :watch c.n
watching c.n — :continue and the run stops when it changes
(dbg) :continue
⏯ watchpoint: c.n changed 5 → 6
⏸ paused in bump | c = Counter{n:6}
```

**When to use it:** the hardest debugging question — *who changed this?* Set a
watchpoint on the field that ends up wrong and let the program run; it stops you at the
write that did it, instead of you stepping line by line hunting for it. Watch a
**heap** value (a struct field or vector element) — those persist; a bare local lives
in a stack slot and isn't watchable. (Watching a whole record, or breaking only when
the new value matches a condition, are planned.)

Breakpoints persist across calls until `:break clear`. Any line of a function body
is a valid breakpoint — `:break <fn>` stops at the first body line, `<fn>:<line>` at
a specific one.

## Debugging a file

You don't have to retype your code into the REPL to debug it. Point the debugger
straight at a source file:

```loft
loft debug prog.loft:12
```

This loads `prog.loft`, sets a breakpoint at **line 12**, runs `main()`, and stops
there — dropping you into the exact same `(dbg)` prompt as above (inspect, edit,
step, `:undo`, `:continue`). Inside a real file line numbers are unique, so you name
the line directly (no need for the REPL's `<fn>:<line>` form).

```loft
$ loft debug prog.loft:12
loft debugger — break at prog.loft:12.  :help for commands, :continue to run, :quit to exit
⏸ paused in update | dt = 0.016, entity = Entity{x:4,y:7}
(dbg) entity.x
4
(dbg) entity.x = 0          # try a what-if
(dbg) :continue
▶ resumed — run finished
```

**When to use it:** this is the everyday way to debug — you have a `.loft` program
and a line you're suspicious of. Set the breakpoint there, run, and poke at the live
state. If the line isn't breakable (a blank line, a bare `}`), the debugger says so
and lists the lines that are.

**Notes.** The program is entered through `main()`. To read a local whose name is a
step verb (`n`, `s`, `c`, `u`, …), use `:vars` or an expression (`n + 0`) — a bare
verb word steps. The breakpoint is scoped to *your* file, so the standard library's
identical line numbers are never caught.

## Introspection without the REPL

`loft introspect <file>` prints the same views for a whole program:

```
$ loft introspect myprogram.loft          # bytecode + Rust + slots + types
$ loft introspect --show-bytecode prog.loft   # one section only
$ loft introspect --fn n_main prog.loft       # one function only
$ loft introspect --all-fns prog.loft         # include the standard library
```

By default only your own functions are shown; `--all-fns` adds the standard
library. Sub-flags select one section (`--show-bytecode`, `--show-rust`,
`--show-slots`, `--show-types`) or write a section to a file (`--bytecode-out`,
`--rust-out`, …). This replaces the older `LOFT_LOG=…` dump route for everyday
use; `LOFT_LOG` still works for live execution traces (see
[RUNNING_TESTS.md](RUNNING_TESTS.md) § LogConfig).

## How session state works (and its limits)

When you bind a name to a value, the REPL runs the right-hand side **once** and
snapshots the value — for **every** type (numbers, text, structs, vectors,
enums) — so a side effect (printing, reading input) in the binding happens a
single time, however often you read the name later (`name = read_line()` prompts
once).  A typed binding (`x: float = 1`) keeps the type it declares.

An input that changes a bound variable **in place** — `x += 1`, `s.a = 9`,
`w.sort()`, `w[0] = 7`, a call that takes it by `&` — is kept the same way: it
runs once, and each variable it changed is snapshotted again, so the next input
reads the new value and a side effect in it does not repeat.  An input that
only reads a variable records nothing.  A value with no literal form (a bare
`hash`) cannot be snapshotted, so an input that writes one is kept as source
and replayed instead; a side effect in that input repeats on a later read.

### The session store — values live in a store, not in replayed source

A bound value is **materialized into a session store** and the environment maps
`name → (type, record)`. Observing a name reads that record: no generation is
compiled and the accumulated body is not replayed, so observe cost stops growing
with session length. On by default; `LOFT_NO_STORE_OBSERVE` opts out.

What that buys, and what it does not:

- **Observing is O(1) in session length** for a store-resident binding. A `text`
  binding is the exception — it is stored as the single-element `vector<text>`
  the @P293 work-around builds, and the display renderer quotes a vector's text
  elements, so text observes still fall back to the replay. Correct output,
  no speed win.
- **A re-bind releases the old record** (`n = n + 1` does not grow the store).
  Nothing else holds a reference into the session store, which is what makes
  freeing on replace safe. `:reset` drops the store entirely — it rides the
  session object.
- **Resume still re-runs your bindings**, so a non-deterministic result
  (`random()`, `now()`) is recomputed. The exact value-for-value path exists as
  an on-disk **session image** (`save_session_image` / `load_session_image`,
  gated by a storage-layout hash so a stale or cross-build image falls back
  rather than miscomputes) but is not yet wired into auto-resume.

### Reading values from an embedder

`ReplSession::eval_value(line)` evaluates one line and **returns** its rendered
value instead of printing it — `Some(text)` for an expression, `None` for a
binding or definition, `Err(diagnostics)` on a fault. A bare name is answered
from the session store, so it compiles nothing.

## See also

- [LOFT.md](LOFT.md) — the language the REPL evaluates.
- [STDLIB.md](STDLIB.md) — the functions available in every session.
- [plans/12-repl-and-introspection/](plans/12-repl-and-introspection) — the
  design and per-phase notes for this work.
