# A record result on the first pass — what the Rust front end decides (@PLN91)

What to mirror for a function whose result is a record that is no literal: the port's refusal
"a function returning a record from something other than a literal".  The probe for the first
shape is [probes/record-tail-default.loft](probes/record-tail-default.loft).

## Two decisions in a row

Every such tail goes through `classify_reference_delivery` and then `classify_ret_promotion`
(`src/parser/control.rs`), on BOTH passes, and each prints a `route` line.  The first pass is
compared on `fn-end <fn> ret=<type> attrs=<n>:<name>` only: the result's type with its dep.
The type spells the dep through the VARIABLE of that number (`["__retbuf"]`); `attrs` names the
attribute, and that name is what tells the cases apart.

| selector | promotion | `attrs` names | the tail |
|---|---|---|---|
| `MaterializeView` | `Rename` | the work ref (`__ref_K`) | `<collection read> ?? RecordLiteral` where the read is of a store the function holds: a call's result, a field of it, a local |
| `Rename` | `Bind { substitute: false }` | `__retbuf` | the same tail where the subject is a PARAMETER's field: nothing to copy |
| `MaterializeView` | `Bind { substitute: true }` | `__retbuf` | a LOCAL that borrows (`class`, `chain`), or a fn-ref call |
| `Rename` | `Rename` | the local | a local, an `if` or a `match` whose value is the function's own |
| `Rename` | `MergeAttr` | the local, or the parameters | the same, where the attribute exists already |

The verdict differs between the passes by rule: a site that is `Rename` on pass 1 is `MergeAttr`
on pass 2 (the attribute exists by then), or `Bind { substitute: true }` with a later work ref.

The testing side keeps the functions of each class with both sides' lines per tip.

## Reading a tail's routes in the trace

A function's TAIL routes are printed with the lexer on the next item's first token.  For
`fn nullable(…) { … }` on line 107 the `classify_*` lines are at `108:3`.  So at `<line>:3` of a
one-line function those lines belong to the function ABOVE it.

Rust's own `LOFT_TRACE_RETPROMO=1` prints the same verdicts with the function's name
(`[retpromo] fn=<fn> … pass=<n> … => <verdict>`): the second instrument to check a reading
against.
