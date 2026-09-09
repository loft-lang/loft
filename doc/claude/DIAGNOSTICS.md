<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# Diagnostic codes

Every diagnostic loft can raise should carry a **code** — a kebab-case slug printed in
brackets after the level:

```
advice[avoidable-copy]: copy of vector<integer> — `src` is still used after this point …
```

The code is the **frozen identity**; the prose is free to improve (@PLN102 arc-E E1). That
split is what lets four consumers share one handle:

- a **reader** searches for it;
- a **fix** attaches to it, rather than to a message string that drifts (@PLN131);
- **this file** is named by it — the door a fix's concept opens onto;
- `grep -rn "<code>"` finds the emitter, its tests and its documentation together.

loft uses **slugs, not numbers** (`avoidable-copy`, not `E0142`). A slug is self-describing
*and* greppable; a number means nothing until you have found the lookup table.

> **A code is a public surface, frozen once emitted.** Renaming one breaks every link and
> search that ever pointed at it, so the naming pass matters more than the edit. Assigning a
> code to a site that had none is additive and always allowed.

## The codes

The **fix** column says what `--explain` offers at that code: `M` a mechanical fix, `C` a
conditional one (each is one fix line), and the concept door they open onto.

| code | level | what it says | what to write instead | fix |
|---|---|---|---|---|
| `avoidable-copy` | advice | A structure was deep-copied because its source is still used after the copy site, so it could not be moved. | Build the value in place, or stop using the source afterwards. Both take the copy to zero — see `@F106`. | M C · `@F106` |
| `double-move` | warning | One droppable value was handed to TWO owners — a container, or a plain whole-value copy `t = s`, which owns what it copies — and each owner releases what it owns, so the value is released twice. Fires only where both hand-offs certainly run (@PLN139). | Build a second value for the second container, or give it to one container only and read it back from there. | C C · `@F106` |
| `lost-write` | warning | A write landed in a COPY (C86) and reached nothing. Two shapes: a local mutated but never read, and (loft#894) a call that writes through a by-value struct parameter given a value RETURNED by another call — `hurt(first(s), …)`, where the returned copy is freed at the end of the statement while `hurt(s.es[0] ?? E {}, …)` lands. | Bind a live reference with `&` for write-through; pass the element itself rather than a call returning it; or read the copy back if a copy was intended. | C C · `@F21` `@F106` |
| `text-parse-may-fail` | error | A text parsed `as <numeric>` can fail, and the result was typed non-null. | `?? <default>` for a fallback, `(… as T?)?` for the type's default, or `as T?` for a checked cast. | C C C · `@F2` `@F96` `@F5` |
| `cast-constant-out-of-range` | error | A constant does not fit the type it is bare-cast to, and a bare cast asserts that it does. | `?? <default>` for a fallback, or `as T?` for a checked cast. | C C · `@F2` `@F5` |
| `format-unescaped-brace` | error | A literal `}` inside a format string, where `}` closes a hole. | Write it `}}`. | M · `@F35` |
| `format-unclosed-hole` | error | A literal `{` inside a format string, where `{` opens a hole nothing closes — a hole holds code, and code stops at the end of its line, so a `{` with no `}` after it on that line can never close. | Write it `{{`. | M · `@F35` |
| `coalesce-default-type-mismatch` | error | A `??` default is not assignable where the value's type is expected. | Cast the default, or give it a matching type. | C · `@F2` |
| `shift-amount-out-of-range` | error | A constant shift outside `0..=63`, which has no defined result. | Shift by an amount inside the range. | C C · `@F37` `@F2` |
| `c-binding-not-interpretable` | error | A `#c` binding takes more C arguments than the contract covers (`MAX_C_ARITY`, 32) — refused on BOTH backends, at the declaration in your own code and at any call site. | Wrap it in an ANSI-C shim taking at most 32 parameters. | M · `@F92` |
| `superseded-call` | advice | A call to a `#superseded` symbol, from source you own. The old form keeps working — this is a signpost, never a removal. | Call the successor instead. | M · `@F109` |
| `unknown-function` | error | A call to a name that does not resolve, where a similarly-spelled function exists. | Rename to the suggested function. | M · `@F16` |
| `unknown-field` | error | A field or method that does not exist on the type, where a similarly-spelled one does. | Rename to the suggested field. | M · `@F12` |
| `unknown-variable` | error | A name that is not in scope, where a similarly-spelled binding exists. | Rename to the suggested binding. | M · `@F16` |
| `redundant-coalesce` | warning | A `??` whose left side is a non-null field — the default can never be used. | Delete the `?? <default>`. | M · `@F2` |
| `redundant-default-fallback` | warning | A `?` on a non-null field — the type default can never be used. | Delete the `?`. | M · `@F96` |
| `redundant-null-check` | warning | A comparison against `null` on a non-null field. Its answer is fixed UNLESS a null reached the slot anyway — C85 writes the reserved sentinel there on an integer overflow (and a `float` can hold a `NaN`), and C80 answers an out-of-range read with null while the index expression stays typed non-null. So both fixes are conditional (loft#1297). | Delete the check, or compare the value you meant. | C C · `@F1` |
| `redundant-null-negation` | warning | `!x` on a value whose type keeps no code for null — `!` tests presence, so it is always false. Asked of the TYPE (`IntegerSpec::non_null_reads_null`), not of the `not null` flag, which only a field declaration sets: until @PLN152 step 5 it answered `if !f.i` and stayed silent on `if !a` over a `u8` LOCAL and `if !v[0]` over a `vector<u8>`, which are the same type and equally always false. It is also @PLN152's adjacency gate — silent where an `if !place` directly follows a store to that narrow place, because there it reads whether the store fit. | Compare the value (`x == 0`) if that is what you meant, or move the test to the line after the store if you meant *did this fit*. | C · `@F1` |
| `dead-assignment` | warning | A local is overwritten before it is read. Silent on a variable a closure CAPTURES — the capture is a read the check cannot see, so `s = 10; f = fn() { s }; s = 20;` would otherwise advertise deleting the line that supplies `f`'s answer. | Delete the assignment, or read it before the next. | C · `@F100` |
| `never-read` | warning | A local or parameter is never read. | Delete it — for a parameter, that changes the signature. | C · `@F100` |
| `shadowed-by-method` | warning | A library's free function is unreachable by its bare name: a same-named METHOD on its first argument's type takes the call (`find_fn` resolves `t_<τ>_f` before `n_f`), from the declaring file, the library's other modules, and any consumer that imported it. The definition stays legal — @PLN102 C97 module-scopes it — so `lib::f` still reaches it (loft#940). | Rename it, or call it qualified as `<package>::f(…)`. | C C · `@F16` |
| `upper-case-local` | advice | An UPPER_CASE local — that style is reserved for constants. | Declare it `const`, or rename to lower_case. | C M · `@F18` |
| `unreachable-code` | warning | Statements after a terminator can never run. | Delete them. | M · `@F16` |
| `unreachable-match-arm` | warning | A match arm an earlier arm already matches. | Delete the arm. | M · `@F29` |
| `empty-parallel-block` | warning | A `parallel` block with no arms. | Delete it. | M · `@F33` |
| `text-slice-char-bound` | warning | A text slice ends at `len(text)` — a character count where a byte offset is wanted, so it stops short on multi-byte text. | `size(text)` for the byte length, or `text[i..]` for the rest. | M M · `@F97` |
| `text-index-char-bound` | warning | An index walks `0..len(text)` (characters) but reads bytes — silent on ASCII, wrong elsewhere. | Iterate `for c in text`, or walk bytes with `0..size(text)`. | M C · `@F97` |
| `index-bounds-other-vector` | warning | A loop bounded by `len()` of one vector indexes a different one — typed non-null, reads null on overrun. Opt-in (`LOFT_LINT_STRICT_INDEX`). | Bound the loop by the vector it indexes. | C · `@F6` |
| `function-complexity` | advice | A function's control-flow complexity passed the nudge, naming its deepest nesting line. | Lift the innermost part into its own function. | C · `@F16` |
| `too-many-parameters` | advice | A function takes many required parameters — every caller has to get them all right, in order. Silent on a `#c` binding, whose parameter list is the C function's: a struct cannot cross that boundary and a default does not change the declared C arity, so both cures are impossible there. | Group the ones that travel together into a struct, or give the optional ones defaults. | C C · `@F12` `@F17` |
| `trailing-boolean-parameters` | advice | A function ends with booleans, so a call reads `f(…, true, false)`. | Give them defaults so callers pass only what they change. | M · `@F17` |
| `omitted-field-zero` | advice | A struct literal names some fields and leaves another out, so that one takes its type's zero and nothing in the declaration chose it. Quiet on a field with a declared default, a nullable field, and a bare `S {}` — each of those says what it means. | Declare the field's default on the type, or write it at this literal. | C C · `@F12` |
| `variant-overwritten-binding` | warning | A `match` / `is` PAYLOAD binding is still read after the subject's PLACE is given a DIFFERENT variant (loft#1397). `(B-Disturb)` makes overwriting a place not a disturbance, so the binding keeps naming the same slot and reads the new variant's bytes at the old offsets — the read `variant-field-unchecked` exists to prevent, at the one spelling exempt from it because a per-arm binding was assumed to outlast nothing. Keyed on the ARM's own tag test, so it cannot drift from the parser's numbering. Quiet on an overwrite with the SAME variant (the value is right), on an unrelated field, and on a LOCAL subject, which is a REASSIGNMENT `(B-View)` already materialises and reports. | Copy the payload out before the write — `x = inner;` — and read `x`. | C C · `@F29` |
| `variant-field-unchecked` | warning | A struct-enum field access names a field only SOME variants declare. The tag IS checked (loft#980): on any other variant the read answers null and the write is ignored — but the type does not advertise that null, and an ignored write is a lost write, which is why this gates. Where the receiver is not a place the guard would evaluate it twice, so that access stays unchecked and the message says so. Quiet when EVERY variant declares the field (one shared slot — the case C89 promises works) and for a synthetic `__nullable<S>`. | Reach the payload per-variant with `match` or `is`, whose bindings are scoped to the arm. | C C · `@F29` |
| `linked-group-double-fill` | advice | One struct literal gives records to two members of a linked collection group — two routes to a single record set, so both end up holding everything. Quiet when a member is written `[]` (which is how every group is constructed) and when only one member is filled. | Give each field its own element type, or fill the group through one member. | C C · `@F7` |
| `linked-group-apart` | advice | A linked collection group's members are declared APART — an unrelated field sits between them, in a struct or a struct-enum variant (`{ entities: vector<E>, tick: integer, spawn_index: hash<E[id]> }`). The group is one record set either way; adjacency is the signal, because the idiom is written together while a group nobody intended is two fields added at different times. Quiet on adjacent members, on a pair with no keyed member, and on a library's struct (a consumer cannot rearrange it). | Give one field its own element type, or declare them next to each other. | C C · `@F7` |
| `needless-reference-parameter` | warning | A `&` on a tuple parameter that is never written. | Drop the `&`. | M · `@F21` |
| `needless-const-parameter` | warning | `const` on a primitive parameter that is never modified. | Drop the `const`. | M · `@F18` |
| `slow-reference-parameter` | advice | A `&` parameter that is only read — double-indirect on every access, and field mutation already propagates without it. | Drop the `&` unless you reassign the whole binding. | C · `@F21` |
| `not-null-deprecated` | advice | `not null` is inert — a type is non-null by default now. | Delete it, or write `T?` if the type should allow null. | M C · `@F12` `@F1` |
| `module-name-shadowed` | warning / advice | Two packages declare a module of the same file name and NEITHER of them ships one of its own for this `use`, so it binds whichever the search found and load order decides. Since loft#976 a package that DOES ship `src/<module>.loft` binds its own and never reaches here. **Warning** when the file that won belongs to the ROOT PROJECT and the one that lost to a DEPENDENCY: a published package then answers differently than it does on its own, which is a wrong result (loft#949). **Advice** the other way round. | Warning: rename the project's own `<module>.loft`, since a consumer cannot edit the dependency. Advice: give the package a `src/<module>.loft` of its own, or write `use self::<module>` to say so explicitly. Outside a package there is nothing to qualify with, so there the cure is to rename one file and its `use`. | M · `@F16` |
| `undeclared-dependency` | advice | `use <pkg>` resolved a package from the registry that the project's `loft.toml` never declares, so nothing distinguishes a dependency from a package that happens to be installed on this box — and an undeclared package resolves to the NEWEST installed, so it is not pinned either. Quiet for a bare script with no manifest above it, and for a package parsed out of the registry cache (that manifest is someone else's). | Run `loft install <pkg>`, which records it under `[dependencies]`. | M · `@F55` |
| `lib-flag-outranked` | advice | `use <id>` resolved somewhere other than a `--lib` directory that also provides it: resolution is first-wins, and a project-local `lib/`, a declared dependency and the script's own directory are searched before the flag, so the flag never reaches a name one of those provides — and three measurements of a patched library copy scored the unmodified tree before this said so. Reports the precedence rather than moving it. Quiet when no `--lib` was given, when the winner lies inside the flag's directory, and once per id. | Run from a directory without a `lib/` that provides the name, or put the override where the resolution looks first. `LOFT_NO_LIB_OUTRANKED` silences it. | M · `@F55` |
| `const-reevaluated` | advice | A file-scope `const` is an inlined expression, so its initialiser runs at every reference. | Compute it once in a function that caches. | C · `@F18` |
| `digit-separator-grouping` | warning | Digit separators that are not on thousands boundaries. | Regroup in threes. | C · `@F3` |
| `empty-braces-not-collection` | warning | An empty `{}` where a collection literal was meant. | Write `[]`. | M · `@F6` |
| `divide-by-constant-zero` | warning | Division or modulo by a constant zero — the result is always null. | Divide by a value that can be non-zero. | C · `@F38` |
| `unary-minus-binds-tighter` | warning | `-x ** y` parses as `(-x) ** y` — the sign binds tighter than `**`. | Parenthesise as `-(x ** y)`. | C · `@F37` |
| `read-size-not-element-multiple` | warning | `f#read(n) as vector<T>` counts bytes, and `n` is not a multiple of the element width, so the tail is dropped. | Pass `element_count * <width>`. | M · `@F40` |
| `file-write-width` | warning | `f += <integer>` with no width cast writes 8 bytes; a binary file usually wants an exact width. | Cast to the width you mean (`as i32`, `as u8`, …). | M · `@F40` |
| `persist-bind-through-field` | advice | `store_persist_bind` on a collection reached through a field writes the whole container's store, which will not load back into a bare collection. | Bind a local of the collection's own type. | C · `@F40` |
| `missing-return-path` | warning | A function declared `-> T not null` has a path that returns nothing. Currently unreachable — a hard error fires first. | Return a value on every path, or declare `T?`. | C · `@F16` |
| `package-contract-drifted` | warning | A package was tested against an older loft contract than this one. | Ask its author to republish against the current contract. | C · `@F55` |
| `superseded-unknown-successor` | error | `#superseded "X"` names a symbol that does not exist, so the steer would ship dangling. | Name a real replacement, or drop the attribute. | C C · `@F109` |
| `superseded-not-folded` | warning | A `#superseded` symbol's body never calls its successor, so the steer ships without its fold. | Reimplement the superseded symbol as a shim over the successor. | C C · `@F109` |

**Every code offers a fix**, and `every_pinned_code_offers_a_fix` keeps it that way. When one
genuinely cannot yet, it goes in `FIX_BLOCKED` (`tests/e1_code_set.rs`) with the reason —
a listed exception, because a code that quietly ships without a fix looks exactly like one
that does not need one. That list is currently empty. It last held the `superseded-*` pair,
whose concept is `#superseded` itself: no catalogue entry meant no door, and a fix that links
nowhere is not finished. `@F109` cleared it, exactly as `@F106` had for `move`.

## Fix lines — `--explain`

A diagnostic says what is **wrong**. `--explain` (or `LOFT_EXPLAIN=1`) adds what to write
**instead**:

```
advice[avoidable-copy]: copy of vector<integer> — `src` is still used after this point …
  --> prog.loft:7:0
  |
7 |   h = Holder { v: src };
  | ^
  fix  build the value in place   [move · @F106]
  fix  drop the later use of `src`   needs: `src` is used again at line 8 — you do not need that   [move · @F106]
```

Three homes, no repetition: the diagnostic says what is wrong, the fix says what to write
instead, and the linked feature says why. A fix that re-explains the problem is duplication
the reader pays for every time; one that explains the concept inline has taken the
documentation's job. The concept (`move`) is a **handle** — the searchable noun that opens
the door — and `@F106` is the door.

**The rule cuts both ways, and the second cut is the one that costs.** A message may not
carry its own cure either. Most of these diagnostics used to (*"use `T?` for a checked cast,
or `?? d` for a fallback"*), so `--explain` printed the same advice twice. Moving it out is
what makes the three homes real — but it means a plain run now says only what is wrong, and
a reader who has never heard of `--explain` would simply be told less than before. One line
per RUN closes that:

```
error[cast-constant-out-of-range]: the constant 1e30 is out of range for `integer` — a bare cast asserts the value fits

note: 1 diagnostic above suggests what to write instead — re-run with `--explain`
```

Per **run**, not per diagnostic: a pointer under each one would double the output on a file
with fifty copy notices, which is the noise the opt-in exists to avoid. It disappears once
`--explain` is on — nobody needs to be told about a flag they just used.

**A message may only hand its cure away if a fix is there to catch it.** The copy notice
shows both sides: with a named source it has two fixes and the prose says only what is wrong,
and with no named source it has NO fix — so that branch keeps its resolution inline. The
alternative is a reader left with nothing at all.

**Two tiers, and they gate who may affirm the condition, not whether a fix is clickable:**

| | interactive (one click) | unattended (batch, CI) |
|---|---|---|
| **mechanical** — meaning fixed by the code alone | yes | yes |
| **conditional** — correct only if the stated condition holds | yes, the click IS the affirmation | **never** |

A conditional fix states its condition in its own column rather than inside a sentence,
because a click has to affirm it: the reader must **see** the thing being affirmed, not
extract it from a clause. That is also what gives the CLI and an LSP code action one shared
shape — `title` on the lightbulb, `condition` in the confirm step.

The condition names the surviving use by **line**. "`src` is unused after here" sends a
reader hunting; "`src` is used again at line 8" is affirmable in a second, and that
difference is the whole point of the field.

**Not every diagnostic has a mechanical fix.** The append shape (`dst.data += src.data`) has
no build-in-place rewrite at all, so a fix set is shape-dependent; assuming otherwise ships a
fix that does not exist. And a site whose condition an author can see is false should offer
**nothing** — suppressing a bad suggestion matters more than emitting a good one, because
credibility is what makes the click safe.

Four of the codes show what that costs in practice. `coalesce-default-type-mismatch` has an
obvious-looking rewrite — cast the default — that is **not** offered as an `edit`, because
`"x" as integer` is a text parse that can fail, so synthesising it would answer one error
with another. `c-binding-not-interpretable` offers one fix and does not spell it: it is C the
compiler cannot write. (It offered a second — "run it on `--native`" — until @PLN128 arc C
made the ceiling apply to both backends. The code NAME still says "interpretable" and now
reads narrow, but a code is a frozen public surface, so the row moved and the name did
not.) `lost-write`'s two
ways out are BOTH conditional — the analysis proves the write is lost, never which of the
two the author meant, and a mechanical tier there would be a guess wearing the safe label.

`superseded-unknown-successor` withholds an `edit` for a reason worth keeping in view, since
it applies to any deletion fix: "drop the `#superseded` attribute" is a rewrite the compiler
knows exactly, but the diagnostic sits at the **definition**, not at the attribute's span —
so an `edit` there would tell an applier to delete the function. A fix may only spell an edit
it can also **place**.

**Ranking is on SOUNDNESS first; teaching is the tiebreak between fixes that both hold.**
Two codes make the distinction concrete.

`shift-amount-out-of-range` — `?? <default>` teaches an idiom and "use an amount inside
`0..=63`" teaches nothing, but a constant out-of-range shift is nearly always a wrong amount,
so the escape ranks second. The better lesson does not get to paper over a bug.

The two cast codes are the sharper case, and they cost a tier. `as τ?` is the idiom that
generalises, and it makes the expression **nullable** — which a target declared non-null
rejects, so `x: integer = "5" as integer?` does not compile. The parser cannot see that
target: applying the fix changes what pass 1 INFERS, so the same source that reads
`x: integer` before the rewrite reads `x: integer?` after, and only a re-parse knows. That
makes the checked cast **conditional**, not mechanical — its meaning is not settled by the
code alone — and it ranks below the discharging forms that hold wherever the diagnostic
fires. `loft fix` confirms it either way: `verified — yours to accept` where it holds,
`REJECTED` where it does not.

This is the general shape, and it is worth stating once: **a fix's tier is a property of the
evidence, not of how short the rewrite looks.** `lost-write`'s `d = &s.items` is one token
and still conditional, because the analysis proves the write is lost and never which
resolution was meant.

`--explain` never applies anything. `loft fix` is where a fix gets checked and written.

## Checking a fix by running it — `loft fix`

The compiler holds the analysis that raised the diagnostic, so a candidate rewrite can be
**applied to an in-memory copy and the analysis re-run**. That is what an IDE quick-fix
historically could not do, and it turns "this fix is sound" from an assertion into a
measurement.

```
$ loft fix prog.loft
prog.loft
  prog.loft:2  double the brace        [verified]
  prog.loft:7  make the cast checked   [REJECTED (the rewrite introduces an error or a warning)]

$ loft fix --apply prog.loft
prog.loft
  prog.loft:2  double the brace        [applied]
  wrote 1 fix(es) to prog.loft
```

A fix `Clears` only when the diagnostic's **code** is gone from the re-analysis *and* no
error appeared that the original did not have. Both halves are needed: a rewrite that
silences one error by causing another is not a fix, and that is exactly what a
pattern-matched suggestion produces. Matching on the code rather than the message is why the
code index landed first — prose is free to change, the code is not.

`--apply` writes a fix only when all three hold:

| gate | rejects |
|---|---|
| `Mechanical` | a fix whose correctness rests on a condition — an unattended run has nobody to affirm it |
| spells an `edit` | a fix that knows the rewrite but not where it goes |
| verifies | a fix that does not clear its own diagnostic, or that breaks something else |

**How much the second gate rejects, today.** `spells an edit` is not a rare exclusion — it is
the one that decides the whole of `loft fix`'s reach. Of the 25 codes marked `M` above,
`loft fix` acts on six: `unknown-function`, `unknown-field`, `unknown-variable`,
`format-unescaped-brace`, `superseded-call` and `redundant-coalesce`.
(`cast-constant-out-of-range` and `text-parse-may-fail` carry edits too, on their conditional
tier.) The rest are omissions rather than decisions, and several are pure deletions at a span
the compiler already knows — `unreachable-code`, `empty-parallel-block`,
`needless-const-parameter`.

`redundant-coalesce` is the first WARNING-level fix in that list, and how it got there is the
pattern for the rest (loft#1003). Its blocker was real — *"the diagnostic fires BEFORE the
default is parsed, so its end is not yet known at the emit site"* — but that is a reason to
spell the edit LATER, not a reason to have none. The notice keeps its own position and
`Diagnostics::set_fix_edit` attaches the span once the default has an end. Every remaining row
that gives the same reason can close the same way.

⚠ **And it changed how a fix is VERIFIED.** The check used to ask whether ANY diagnostic with
this code remained, which made two instances mask each other: a file with two redundant `??`s
verified neither, because whichever fix was applied the other still answered yes. That is not a
hypothetical — one instance is a demo, two is what real code looks like — and it only surfaced
once a code that fires OFTEN carried an edit. It is a count now: a fix clears when its tally
drops by one, whichever instance it was.
That is now a GATE rather than a note. `every_mechanical_fix_is_applicable_or_listed`
(`tests/e1_code_set.rs`) drives `loft fix` itself over each code's trigger program and requires
that a code whose `--explain` output offers a fix with no `needs:` clause is one `loft fix` can
name — or carries a row in `EDIT_BLOCKED` saying what stops it. Both directions are red: a
missing row, and a row for a code that has since graduated. The twenty above are that list, each
with its reason, so a missing edit is a decision on record instead of a silence; several say
outright that they SHOULD carry an edit and name what is missing (a span the emit site has not
captured). `redundant-coalesce` graduated off that list first and is the worked example for
the others.

The scan is scoped to each code's own diagnostic block, from its header to the next. A trigger
program often reports more than the code it was written for — the `double-move` probe also
raises `avoidable-copy` — and reading the whole output attributes one code's mechanical fix to
another, which is exactly how the first version of this gate failed on a code already listed.

`superseded-call` is worth naming separately, because it is the only advice with an edit and
it is REJECTED every time it fires. The edit is a bare rename, and the stdlib's only
`#superseded` symbol has a successor of different arity — `sum_of(v)` is a shim over
`sum(v, 0)`, and `sum` has no default for `init`, so the renamed call does not compile. The
verification is right; the edit is under-specified.

**The program is never run.** The design asked for a behaviour comparison across both
backends; that would mean executing the author's code as a side effect of their asking what
to write — code that may write files, take a network turn, or not terminate. Verification is
static, and stops where acting on the author's behalf would begin.

`text-parse-may-fail` is the standing example of the third gate paying for itself.
`x: integer = "5" as integer` is offered the checked cast like any other failing parse, and
`as integer?` there yields `integer?` into a non-null slot — plausible, and refused by the
measurement. The same fix verifies where the target is not annotated, which is why it is
CONDITIONAL and ranked last rather than removed: it is the right rewrite most of the time,
and the author can tell in a second whether their own declaration allows it.

## In an editor

Three surfaces, and they carry different halves:

| LSP field | what it carries |
|---|---|
| `codeAction` | a quick-fix per fix that spells an `edit` — both tiers, since a click IS the affirmation. A conditional fix's title carries `— only if …` so the condition cannot be missed, and only mechanical ones are `isPreferred`, because a "fix all" gesture has nobody reading. |
| `relatedInformation` | **every** fix — title, condition, concept and door — as text under the diagnostic |
| `codeDescription` | the code's row in this file, as a link the editor renders on the code itself |
| `source.fixAll` | one edit applying every mechanical, VERIFIED fix — delegated to `fix_apply::apply_fixes`, the same function behind `loft fix --apply`, so the two lanes cannot drift |

**`relatedInformation` is the one that matters most, and it is easy to think redundant.** A
quick-fix needs an `edit`, and only 5 of 62 fixes have one — the rest name a rewrite the
compiler cannot place. Without this row an editor shows a message that, since the messages
stopped carrying their own cure, deliberately no longer says what to write: the cure would
live in the CLI alone, and the editor would be strictly worse off than before the trim.

The `codeDescription` link targets `main`, so it resolves from a release and 404s from a
branch that has not merged this file yet. That coupling is deliberate — a user reaches the
binary through a release cut from `main`, the same commit that carries this page.

**An UNMASKED error is not one the fix caused.** `parse_source` returns early when pass 1
errors, so a truncated parse reports no pass-2 diagnostic at all — casts, shifts, most
semantic lints. Fixing the pass-1 blocker lets the next parse reach them, and a plain
set-difference reads every one as damage the rewrite did. That made `--apply` and fix-all
refuse any file whose fix was not its only error.

Verification therefore compares the two parses only when they are **comparable**: when the
original got as far as the rewrite did. Where it did not, the fix is judged on its own
diagnostic alone — it cleared the blocker, and what the deeper pass then finds was always
there. Which is what a person does: fix the syntax error, then read the type errors.

Judging this by POSITION would have been wrong, and measuring said so before any code
changed: an unescaped brace hides a bad cast three lines below it *and* another two lines
above. The mechanism is the phase, not the line.

The gate still bites where a rewrite genuinely breaks something, because that failure lands
in the same phase the original reached: `x: integer = "5" as integer?` is still refused.
`a_genuinely_broken_rewrite_is_still_refused` is what keeps "ignore unmasked errors" from
becoming "ignore all errors".

**Errors get codes when they get FIXES.** The did-you-mean family is the model: each site
already computed a replacement and knew where the name sat, so the fix was a rename with a
real span — and coding them was justified by that, not by a coverage number. 500-odd parse
and type errors remain uncoded on purpose; a code is frozen the moment it ships, and minting
one for a message that will never carry a fix buys a permanent name for nothing.

**The spelling is a guess; the re-parse is what makes it a measurement.** A suggestion is
chosen by edit distance, which knows nothing about arity or type — `helpr` → `helper` can be
the right spelling and the wrong call. Verification catches exactly that, and it is the
reason these can be `Mechanical` and written unattended: a pattern match alone would have
edited the file and left it broken. It is also why the family reached the LSP years before
it could be applied — a quickfix a human clicks is a different risk from one a
fix-on-save applies.

## Who a diagnostic is addressed to

There are **two independent axes**, and a new diagnostic answers both.

| axis | question | decides | mechanism |
|---|---|---|---|
| **tier** | can ignoring it produce a WRONG RESULT? | does it gate CI | `Level::Warning` vs `Level::Advice` |
| **reach** | who can act on the cure? | who sees it at all | `Diagnostics::reaches_author` |

> **A diagnostic reaches only whoever can act on its cure.**

Every `warning` and `advice` loft emits names a cure that is an edit to the code it points
at — rename the field, declare the default, split the function. Pointing one at a reader who
cannot make that edit is noise by construction, and noise that reads as *their* defect: the
Parser chapter of the reference, whose whole content is four `parser::parse` calls, printed
**eleven** notes about the internals of two libraries the reader did not write (loft#1260).

**Nothing to do per lint.** The gate sits in `Diagnostics::add_at_coded`, the one place every
route reaches — the `diagnostic!` / `diagnostic_at!` macros, and the post-scope lints that
call `add_at_coded` directly with a `&mut Diagnostics`. A new lint is covered by existing.
Errors are never dropped: a program that will not run has to say so whoever is reading.

**The scope is the PROJECT, not the entry file**, and that distinction is the whole
correctness of it. `Data::source_is_owned` answers `source == MAIN_SOURCE`, which is the
entry — so under `loft test` a package's entry is `tests/*.loft` while the code under review
is `src/*.loft`, and an entry-file rule silences a library's lints in exactly the run that
exists to catch them. Measured, and it had already happened: `linked-group-apart` gated
itself that way, fired for a struct in an owned program, and was silent for the same struct
in its own package's test run. The scope is therefore the nearest `loft.toml` above the
entry (`resolution_scope::project_root`); with no manifest it is the entry's own
DIRECTORY — `main.loft` plus the modules beside it is an ordinary program, and scoping to
the single entry file repeats the same mistake one level down. Not its subdirectories: a
vendored dependency goes in a `lib/` below, which is not beside anything.

**`warning` gets the same answer as `advice` here**, which is a decision rather than a
consequence. A consumer who cannot edit a dependency is not helped by learning it has a
lint, only told that something they cannot fix is wrong — and loft is meant to be boring
([GOALS.md](GOALS.md)). Its own author still sees it and `LOFT_DENY_WARNINGS=1` still gates
on it there, because when the author builds the library the library IS the project. The two
alternatives were considered and rejected: *attribute it but keep printing it* makes the
noise polite rather than actionable, and *keep it for a local path dependency* confuses
"can write the bytes" with "owns the code" — the dogfood doctrine forbids editing a
dependency's tree either way.

So: **do not add an ownership test at a lint site.** One home, or the argument gets
re-litigated per lint and the next author guesses.

**The consequence for this repo, stated plainly.** `lib/*.loft` are libraries with no
`loft.toml` of their own, and the repo root has none either — so when a doc chapter or a
guard `use`s one, its lints are now addressed to nobody and are dropped. That is the right
answer for a consumer and a loss for us, who can edit them. The channel that replaces it is
to make the library the ENTRY: `loft --interpret lib/parser.loft` prints its own four, and
the LSP does the same for the file being edited. The durable fix is for these to become
packages with a manifest and their own CI, at which point their `loft test` sees everything.

## How a diagnostic spells a type

**A diagnostic spells a type the way its reader could have written it.** The section above
decides WHO a message reaches; this decides what it says once it gets there, and the two fail
the same way — a message addressed correctly but written in a notation the reader has never
seen is a message they cannot act on.

`Type` answers two different questions and has two functions for them:

| | |
|---|---|
| `Type::name` | **the schema key** — *which type is this?* `typedef.rs` builds wrapper types from it (`main_vector<…>`) and `state` looks stores up by it, so re-spelling a keyed type here RE-IDENTIFIES it: generated `init()` replays a different type order and the emitted Rust references a temp no line binds (rustc E0425, `tests/lazy_sql_source.rs`). |
| `Type::source_name` | **the source spelling** — *what did they write?* This is the one a message asks for. |

They differ at the keyed collections and nowhere else. `name` renders the key list with
`{:?}`, so `index<Rec[id]>` comes out `index<Rec,[("id", true)]>` — a Rust tuple and a boolean
whose meaning (ascending) has no spelling in the language. That is not a rendering of what the
author wrote; it is a notation they have never seen, and a refusal that names it sends its
reader looking for a type that is not in their program.

**Both are one body.** `render(data, source)` carries the flag through its own recursion, so a
type is spelled one way all the way down. Holding them apart as two match statements is what
drifted three times (loft#956, loft#1434, loft#1449) — and never at a keyed arm, always at a
CONSTRUCTOR that had not learned to recurse. `source_name` had arms for `Optional` and `&`, so
`hash<It[k]>?` read right while `fn(&hash<It[k]>)` rendered its parameters through the schema
key. One body makes the arms exhaustive: an arm either recurses or it is a leaf, and a leaf
reads the same under both.

**`scripts/diagnostic_spelling.py check` is the gate** (`make ci`, via
`tests/doc_hygiene.rs`). It fails on a `Type::name(data)` render inside a diagnostic emitter —
`diagnostic!`, `diagnostic_at!`, `specific!`, or a direct `diagnostic_format(…)` — and on ANY
such render in `src/parser/` or `src/variables/`, the layers that talk to the author. A render
that genuinely means the schema key marks its line `// schema-key — <why>`; there are six, and
they are type IDENTITY (a `__tuple<…>` comparison, a `t_<LEN><Type>_` method name) or a
developer trace behind `LOFT_TRACE_UNWRAP`.

⚠ **The marker goes on the RENDER'S OWN LINE**, not in a comment above it — the check reads the
line the call sits on. That is the first mistake a reader makes, and it is the one an author
writing a careful explanation makes by default, so the failure message says it too.

The wider rule is why the gate is not span-only. A span cannot see `let nm = tp.name(data);`
on the line above the `diagnostic!` that interpolates it, nor a helper called from inside one
— and that is where the sites actually were: after every in-span render in those two layers
had been converted, 20 of the 26 left were still user-facing, and one message carried both
spellings three lines apart. **The gate exists because nothing else can fail here.** An
unreadable message leaves the compiler right and the program wrong, so no test goes red and
only the author pays; without a check, the next pass converts the sites someone happened to
have a symptom for and leaves a fourth residue.

**And the error-message BASELINES are not that check, which is measurable rather than
argued.** The eight conversions found on the joined tree changed no golden output at all — so
the locked-in baselines contain no case that renders a keyed collection, and every one of them
would have stayed green through the whole drift. That is a gap in the golden set worth closing
on its own; it is also the reason this gate is a source-side check rather than an output-side
one. A baseline can only pin a message someone thought to write down, and the sites that get a
type's spelling wrong are exactly the ones nobody had a symptom for.

## Adding a code

1. Emit through `Diagnostics::add_at_coded` (or `diagnostic!(… code = "…", …)`), never the
   uncoded `add_at`. Write the message as what is **wrong** — no "use X instead" clause; that
   is the fix's job, and a message carrying both is the duplication above.
2. Add the row to the table above **in the same change**. A code with nothing to grep to is
   the same dead door as a concept with no catalogue entry.
3. Attach the resolution with `Diagnostics::fix_last`, and give the concept a `@F` entry that
   exists. This is not optional: `every_pinned_code_offers_a_fix` fails a code that renders
   none, and `every_offered_door_resolves_to_a_catalogue_entry` fails a `@F` that is not in
   the catalogue. If the fix genuinely cannot be offered yet, add a row to `FIX_BLOCKED`
   naming what blocks it — a listed exception, never a silent one.
4. **Point the caret with `diagnostic_at!` whenever detection happens after parsing.**
   `diagnostic!` uses the lexer's CURRENT cursor, which is only right while the offending
   construct is still under it. A whole-function judgement (complexity, parameter count,
   trailing booleans) needs the whole body first, and by then the cursor sits on the NEXT
   definition — so all three of those pointed at the following `fn` while the prose named
   this one, which reads as advice about a function that is fine (loft#815). Pass the
   definition's own `position`. Same rule for anything checked after its expression is
   consumed. Guard: the golden case
   `tests/error_messages/cases/48_advice_points_at_the_function_it_names.loft`, whose
   fixture deliberately ends in a `next_function_marker` no caret may land on.

5. **A seek to a diagnostic site is not free — it moves what every LATER position is read
   from.** `Lexer::to` points the reporting position at a declaration a whole-body pass is
   complaining about; it does **not** move the read cursor, and the tokenizer keeps
   incrementing that reporting position on every physical line it pulls afterwards. So a
   seek left standing shifts the caret of every diagnostic in the rest of the file, the
   `file:line` of a runtime span, and the line the compiler injects into `assert` — all by
   the same constant, which is what makes it read as correct. Measured: a needless-`const`
   parameter made all nineteen assertions of one test file report seven lines early, so a
   failure printed **another** assert's source under this one's message (loft#625's
   mechanism, at a site that fix did not reach).

   The seek now ends at the next token scanned from source, so a missing restore costs the
   one diagnostic it was made for. A pass that emits several diagnostics and then keeps
   parsing still restores explicitly (`let p = lexer.at(); … lexer.to(p);`) — a position
   read back with `at()` before that next token still sees the seek. Guard:
   `runtime_warnings.rs::a_seek_to_a_warning_site_does_not_shift_later_positions`; the
   corpus-wide re-measure is `LOFT_TRACE_ASSERTS`
   ([TESTING.md](TESTING.md#the-set-a-suite-runs-is-not-the-set-it-contains-loft_trace_asserts)),
   which reads exactly this injected line.

6. **The cursor is one token AHEAD of what the parser has decided about, and `diagnostic!`
   now attributes to the consumed source when that token has crossed a line.**
   `Lexer::position` is the scan cursor: the END of the token the parser is *holding*. A
   check that can only run once a construct is complete — a write to a `const` parameter, a
   nullable reaching a non-null slot, a capture inside a `parallel` arm — raises with the
   cursor already advanced to the next token. While that token is on the same line both
   answers agree, which is exactly why this looked right for years: loft statements usually
   end in `;`, and the `;` keeps the cursor on the statement's own line. Drop it — which the
   language invites, being expression-oriented — and the caret follows the cursor to
   wherever the next token happens to be:

   ```loft
   fn f(a: const integer) {
     a = 42        // ← the error is here
   }               // ← the caret was HERE, and blank lines between them moved it further
   ```

   `Lexer::report_pos` is the one place that decides this: a diagnostic goes to the end of
   the CONSUMED source (`Lexer::prev_end`) whenever the current token starts on a later
   line, and to the cursor otherwise, so the existing same-line column contract is
   untouched. A deliberate `Lexer::to` seek (item 5) outranks both — that position was
   chosen — and the position is only used when it belongs to the same file.

   **A site that really is about the token the parser is HOLDING says so, at the site.**
   Three do: `'struct' definitions must be at file scope` and the `..hi` open-range refusal
   are raised while looking at the offending keyword, and `unreachable-code` is raised
   holding the first token of the unreachable statement. All three now name their position
   (`Lexer::specific` / `pos_diagnostic_coded`) with a comment saying why — the same shape as the 48 sites already reaching for
   `Lexer::peek_pos`. That claim can only be made by the site; the default cannot guess it,
   and measuring proved it: attributing *every* diagnostic to the consumed source moved 107
   of 248 `parse_errors` fixtures, and the ones that moved were syntax errors about the
   current token, all of them worse.

   The gain was not only the missing line. Two diagnostics were pointing at an entirely
   **different construct**: `circular init dependency` landed on the `fn` after the struct,
   and `Not all code paths return a value — function 'classify'` landed on the function
   *after* `classify`. Two more were on their function's closing `}` — the deferred
   `OpMinInt` arity errors and `Unknown field Point.z` — and now land on the expression.
   `Expect token ;` now points where the `;` belongs rather than at the line below it.

   The corpus-wide re-measure is `scripts/diag_position_audit.py` (a report, not a gate — the
   position twin of `LOFT_TRACE_ASSERTS`): its sharp filter, *the caret sits on a line that is
   nothing but a closing brace*, went **19 → 4 → 0** across two passes.

   Guards: `parse_errors.rs::a_diagnostic_names_its_own_line_{whatever_follows_it,
   with_no_terminator, across_blank_lines}` — one statement in three layouts, asserting
   they AGREE rather than a hand-picked line, since a hand-picked expectation only ever
   pins the layout it was written for. The first cell is the one that always passed; it is
   in the file to show what hid the other two. `a_current_token_diagnostic_still_names_the_
   current_token` pins the opt-out direction. All proven able to fail by disabling
   `report_pos`.

7. **A whole-CONSTRUCT check must still name the part it is about, and `report_pos` cannot
   guess which part.** The default of item 6 is right and not enough for a check that can only
   run once a whole construct is complete: its consumed source genuinely ends at the closing
   brace, so the caret lands there. That is correct and useless — a struct has many fields, and
   *"somewhere in this struct"* is not an answer. Each such site holds a better position:

   | check | now points at | how |
   |---|---|---|
   | `circular init dependency` | the field the cycle STARTS from | the field name's position, threaded through `init_deps` |
   | a generator's discarded tail value | the tail expression | `l[last].span_pos()` — a call is already span-wrapped at its `(` |
   | a tail conversion (narrowing, `not null`) | the tail statement | `block_result`'s `tail_pos`, captured per statement by the block loop |

   Two cost nothing to reach: `tail_pos` was already a parameter of `block_result` and used by
   exactly one check, and the N-Store tail check next to it already read `span_pos()`. Only
   `circular init` needed a new datum. Measure before widening a site: three of the four
   narrowing POSITIONS (assignment, argument, struct-literal field) already named their own
   line — only the return tail did not — so the fixture pins all four, or a later change could
   move the working three unnoticed.

   ⚠ **Seek with `Lexer::to`, end with `Lexer::end_seek` — never with a second `to`.** Seeking
   back leaves `seek_return` pending, and `report_pos` reads a live seek as a deliberate choice
   and stops attributing to the consumed source. So every diagnostic the pass raises AFTER the
   seek silently reverts to the scan cursor. Measured: seeking around the block-tail conversion
   that way sent `Not all code paths return a value — function 'classify'` back onto the
   FOLLOWING function, which is the same defect item 6 had just removed. `missing_return_not_null`
   is the guard that caught it.

## See also

- [COPY_DIAGNOSTICS.md](COPY_DIAGNOSTICS.md) — the copy-vs-borrow model behind `avoidable-copy`.
- [plans/131-suggestions/](plans/131-suggestions/README.md) — @PLN131's closure record:
  what was decided, and what the build learned that the design had not.
- `@F110` — the user-facing capability in the feature catalogue.
- `@F106` — copy and move semantics, the feature the copy fixes open onto.
