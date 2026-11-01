# What clears a warning

A loft warning is meant to point at a construction that makes it go away **and says more than
the code did before**: what the author knows, or what the program may meet.  A change that
only silences the warning — a `?` on a value that should never be null, a dummy default — is
not a fix: it turns a bug the warning saw into a quiet default nobody sees.

This page lists, per warning, the constructions that clear it and when each one is the right
one.  The code index with each warning's exact scope is [DIAGNOSTICS.md § The codes](DIAGNOSTICS.md#the-codes);
the rules the constructions rest on are cited inline.

## A nullable value stored into a non-null slot

> warning: a nullable `τ?` is stored into *the slot* of the non-null type `τ` — it becomes null
> there; …

`(N-Store)` (`formal/types.md`): a `τ?` must be discharged before it lands in a `τ` slot — a
local declared `τ`, a parameter, a field, an element, a return value.  The question to answer
first is **where the null comes from**, because that decides the construction.

### The null comes from an index the program guarantees

`v[i]` reads `τ?` unless `(N-Index)` can trust the index.  The better construction is
usually an index it trusts, which states the guarantee in the code itself:

| you mean | write | why it is trusted |
|---|---|---|
| the last element | `v[-1]` | a constant; one constant beats `v[len(v) - 1]` on the interpreter too |
| every element in turn | `for x in v { … x … }` | the loop variable is the element |
| the element beside another vector's counter | `for x in v { … v[x#index] … }` | the counter of a loop over the SAME vector |
| an element counted from a length you keep | `n = len(v); … v[n - 1]` | a length local: assigned once, `v` written nowhere that can run after it |
| an element after a bounds test | `if i < len(v) { … v[i] … }`, `assert(i < len(v), …); v[i]`, `if i >= len(v) { return … }` | a guard (`(N-Guard)`) |
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
| `tail-call-not-jumped` | Write the recursion as a `while` loop, or pass a part of a parameter instead of a value made for the call.  To keep an ordinary call on purpose, bind its value first: `r = f(…); r`. |
| `text-slice-char-bound` | Slice by bytes: `size(t)` for the byte length, `t[i..]` for the rest.  `t[..-1]` drops a final ONE-BYTE character such as `\r`; a bound inside a multi-byte character moves to that character's end, so it does not drop a final `€`. |
| `shadowed-by-method` | If the method does the same work, delete your function and call the method; otherwise rename it, or call it qualified as `lib::f(…)`. |
| `avoidable-copy` (advice) | Move the line that reads the source again above the copy, or drop that line when its value is never read.  When neither holds the copy is necessary and nothing is reported. |

## Open

- The nullable-store warning's caret marks the slot (the `{` after a return type, the `)` of a
  call) rather than the nullable expression, so it does not yet point at the code to change.
- Its message offers only `?`, `?? <default>` and `match`.  It does not yet name `!` for a
  guaranteed value, an index shape `(N-Index)` trusts, or declaring the slot `τ?`, so the one
  construction it suggests first (`?`) is the one that is most often wrong.
