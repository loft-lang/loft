# @PLN91 — a text `if` whose arm answers a variable, appended to the text a function returns (prepared subject)

**The family:** "an append of a text `if` whose arm answers a variable" (9 sites; 2 programs
held by it alone):
```
a-text-choice-between-slices-is-a-view.loft:18, :28, :52    out += if last { "=" } else { chars[k..k + 1] };
2225-a-text-if-is-one-value-wherever-it-goes.loft:76          out += if out == "" { tw(W { s: s }) } else { " " + tw(W { s: s }) };
```
The port refuses at el_ex_place.loft ~2457.  Its comment says why: the appended local would
BORROW the arm's variable, and the function's tail then hands that borrowing local up, "rules
not carried yet".  The guard fires only where the function answers a text.

**The rule:** a text append COPIES the appended characters into the target.  `OpAppendStackText`
writes into the target's own buffer, so the target borrows nothing from the operand, whichever
arm the `if` takes.

## Rust's form, measured on try bc97fb998

```
fn pick(chars: text, k: integer, last: boolean) -> text { out = ""; out += if last { "=" } else { chars[k..k + 1] }; … out }
  chars: text;  k: integer;  last: boolean;  out: &text hidden          <- `out` renamed onto the returned text's buffer
  -> text["out"]
  {#block: text["out"]
    [17] out: &text = "";
    [18] OpAppendStackText(out, if last {#block: text  "=" }
                                else {#block: text["chars"]  OpGetTextSub(chars, k, OpAddInt(k, 1)) });   <- the if, as one value
    [19] OpAppendStackText(out, if OpLtInt(2, k) {#block: text["chars"]  OpGetTextSub(chars, OpMinInt(k, 1), OpAddInt(k, 1)) }
                                else {#block: text  "-" });
    [20] out }
```
`out` keeps its own type, `&text` / `text["out"]`.  No dep on `chars` reaches it: only the arm
blocks name `["chars"]`, because the slice is a view while it is being appended.

## Each form judged

- **The `if` appended as one value (`OpAppendStackText(out, if …)`), the target keeping its own
  type:** a RULE.  An append copies, so the borrow the port's comment fears does not arise.
- **Each arm block typed by its own value (`text` for a literal, `text["chars"]` for a slice):** a
  RULE (`@FR-B-View`).  A slice is a view of `chars` while it is read.

## What to code

At ~2457, carry the text `if` as the append's operand and leave the target's type alone.  Emit
`OpAppendStackText(<target>, <the if>)` for a `&text` target, the stack form the code above it
already chooses.  Start with a-text-choice-between-slices (`pick`, `quads`, `guarded`: slices
and literals).  2225's arms are calls and a concatenation: each arm builds its own work text
first.  Measure its form before carrying it.

## Probes (the two sole programs)

```
kr-try/tests/scripts/a-text-choice-between-slices-is-a-view.loft:18, :28, :52
kr-try/tests/scripts/2225-a-text-if-is-one-value-wherever-it-goes.loft:76
```
