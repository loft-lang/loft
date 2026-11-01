# What clears a warning

A loft warning is written so that reading it makes you want to switch: it says what is wrong
in the terms of YOUR code, and names a construction that makes it go away **and says more
than the code did before** — what the author knows, or what the program may meet.  A change
that only silences the warning — a `?` on a value that should never be null, a dummy default —
is not a fix: it turns a bug the warning saw into a quiet default nobody sees.  A warning that
cannot name such a construction for the code in front of it is a defect in the warning.

This page lists, per warning, the constructions that clear it and when each one is the right
one.  The code index with each warning's exact scope is [DIAGNOSTICS.md § The codes](DIAGNOSTICS.md#the-codes);
the rules the constructions rest on are cited inline.

## A nullable value stored into a non-null slot

> warning: a nullable `τ?` is stored into *the slot* of the non-null type `τ` — it becomes null
> there; *the construction for this source*

`(N-Store)` (`formal/types.md`): a `τ?` must be discharged before it lands in a `τ` slot — a
local declared `τ`, a parameter, a field, an element, a return value.  The warning reads where
the null comes from and names the construction for that source (`Parser::null_cure`), with
the caret on the expression when it has a position:

| the value | the warning says |
|---|---|
| an element read by an index `(N-Index)` cannot prove | index with a shape it proves (`v[-1]`, the loop's own element, `v[x#index]`, an index tested against `len(v)`), or `!` after the read |
| a keyed lookup (`h[k]`) | handle the miss with `match` or a guard, or `?? <value>` |
| an `if` / `match` with a branch answering `null` | declare the slot `τ?` if null is a real answer; make that branch halt if it cannot happen |
| a call of a function that answers `τ?` | decide what null means here: `!`, a guard or `match`, `?? <value>` — or declare the slot |
| a local that is `τ?`, or a field or computation of one | make the value non-null at the line that gave it (named), or test it here (`if x == null { … }`), or `x!`; a parameter declared `τ?` is told to be declared `τ` when every caller passes one |

Declaring the slot `τ?` is offered only where the slot is the author's to declare — never a
parameter, which belongs to the function called.  `?` is named last, and only as the type's
default when that default is the answer.  The sections below are the reasoning behind each.

### The null comes from an index the program guarantees

`v[i]` reads `τ?` unless `(N-Index)` can trust the index.  The better construction is
usually an index it trusts, which states the guarantee in the code itself:

| you mean | write | why it is trusted |
|---|---|---|
| the last element | `v[-1]` | a constant; one constant beats `v[len(v) - 1]` on the interpreter too |
| every element in turn | `for x in v { … x … }` | the loop variable is the element |
| the element beside another vector's counter | `for x in v { … v[x#index] … }` | the counter of a loop over the SAME vector |
| an element counted from a length you keep | `n = len(v); … v[n - 1]` | a length local: assigned once, `v` written nowhere that can run after it |
| an element after a bounds test | `if i < len(v) { … v[i] … }`, `i < len(v) && v[i] > 0`, `assert(i < len(v), …); v[i]`, `if i < 0 \|\| i >= len(v) { return … }` | a guard (`(N-Guard)`): an `if`, the right side of `&&`/`\|\|`, an `assert`, a guard clause on one test or an `\|\|` of tests |
| everything but the last, as a new vector | `v[..-1]` | a slice is a fresh value, with no element read to fail |

Where no such shape fits but an invariant guarantees the element — a variable number the
program itself handed out, a definition number it just created — write **`v[i]!`**
(`(N-Present)`): present, or a halt that names the expression.  It says *"this cannot be
absent"*, and if the invariant is ever broken the program stops at the read instead of
carrying a null onward.

### The null is a real answer

A value that may genuinely be absent — a lookup that can miss, an optional input, a function
with no answer for some argument — is carried as `τ?` until the place that decides what
absence means:

- **Declare the slot `τ?`.** A function that answers `null` on some path (`_ => null` in a
  `-> float`) declares `-> float?`; a local that may hold nothing is `x: τ? = …`; a field is
  `f: τ?`.  The caller then decides.
- **Decide with a guard** (`(N-Guard)`): `if body == null { return …; }` — after it, `body`
  reads as `τ`.  `assert(x != null, …)` does the same where absence is a bug.
- **Decide with `match`** (`(N-Match)`): `match x { null => …, n: _ => … n … }`.
- **`x ?? d`** when there is a meaningful value to use instead (`(N-Coal)`).
- **`x?`** only when the TYPE's default (`0`, `""`, an empty record) is itself the right
  answer (`(N-Default)`).  It is never the right way to silence a value that should not be
  null — that is `!`.

### The null comes from a field of a nullable read

`v[nr].name` warns about the `.name` because `v[nr]` was `τ?`.  Fix the read, not the field:
one of the index constructions above, or `v[nr]!.name`.

## The other warnings in an ordinary build

| warning | the construction that clears it |
|---|---|
| `never-read` | Delete the local, or the parameter (that changes the signature).  For a parameter an interface makes you keep, name it `_…`. |
| `tail-call-not-jumped` | Pass a part of a parameter instead of a value made for the call (`v.ops[-1]`, not `last(v.ops)`), or write the recursion as a `while` loop.  To keep an ordinary call on purpose — a one-step re-entry — bind its value first: `r = f(…); r`.  The warning stands on the call that blocks the jump. |
| `text-slice-char-bound` | Slice by bytes: `size(t)` for the byte length, `t[i..]` for the rest.  `t[..-1]` drops a final ONE-BYTE character such as `\r`; a bound inside a multi-byte character moves to that character's end, so it does not drop a final `€`. |
| `shadowed-by-method` | If the method does the same work, delete your function and call the method; otherwise rename it, or call it qualified as `lib::f(…)`. |
| `avoidable-copy` (advice) | Move the line that reads the source again above the copy, or drop that line when its value is never read.  When neither holds the copy is necessary and nothing is reported. |

## Open

- A value with no position of its own (a local read, an `if` built by the parser) still puts
  the caret on the slot — the `{` after a return type, the `)` of a call.
