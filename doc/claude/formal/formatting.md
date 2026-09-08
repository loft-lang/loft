<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/formatting.md — semantics for text formatting & interpolation (strict)

**Catalogue:** @F (text formatting), @PLN89 (differential oracle). Behaviour sources:
[LOFT.md § Strings](../LOFT.md), [STDLIB.md](../STDLIB.md); fault-safety is C66 / @P376.

> **Rules then deviations** (see [README](README.md)). This is the relation for **string
> interpolation** — the `"…{e}…"` template — and the **value→text rendering** every scalar and
> heap type obeys. It extends [operational.md](operational.md) (eval order, the null sentinels,
> uncomputable→null) and [heap.md](heap.md) (a struct/vector renders by walking its store). Every
> rule is a **user-visible contract** verified on both backends.

## Notation

Uses [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩`. Write `⟦v⟧_f` for the **text
rendering** of value `v` under an (optional) format spec `f` — the text that an interpolation
`{e:f}` produces once `e` has evaluated to `v`. A **template** is a string literal `"s₀{e₁:f₁}s₁…"`
of interleaved literal runs `sᵢ` and interpolations `{eᵢ:fᵢ}`.

---

## Rules

### Interpolation & escape — `{e}` splices a rendered expression

```
  (F-Interp)  a template "s₀{e₁:f₁}s₁…{eₙ:fₙ}sₙ" evaluates each eᵢ LEFT TO RIGHT
              (operational.md E-Left) and produces the text  s₀ · ⟦v₁⟧_f₁ · s₁ · … · ⟦vₙ⟧_fₙ · sₙ
              (`·` is concatenation).  eᵢ is an ARBITRARY expression — a variable, field access
              `a.b`, index `v[i]`, call `f(x)`, arithmetic — parsed at full language level.
  (F-Escape)  inside a template `{{` denotes a literal `{` and `}}` a literal `}`; a single `{`
              opens an interpolation.  Everything outside `{…}` is copied verbatim.
```

**In words.** `"hi {name}, {a + b} left"` renders the literals unchanged and splices each
`{…}` by evaluating the expression and rendering its value; the pieces are concatenated in source
order. To put a real brace in the output, double it — `"{{lit}}"` renders `{lit}`. The braces
hold *any* expression, not just a bare name (verified: `{42:#x}`, `{col}`, `{a / b}`).

### Desugaring — a template is a text accumulator, one sink for both backends

```
  (F-Desugar)  a template compiles to a fresh TEXT accumulator: clear it, then for each piece
               append the literal run and append ⟦vᵢ⟧_fᵢ; the template's value is that text
               (type `text`).  Both backends route rendering through the SAME functions
               (`ops::format_*`, and the store walker for heap values), so ⟦v⟧_f is
               backend-independent by construction.
```

**In words.** `"…{e}…"` is not a hidden `+`-chain of separate strings; it builds one text buffer
by appending each rendered piece. Because the interpreter and `--native` call the identical
rendering code, the produced text is the same on both — this is what makes formatting a *rules*
doc (a written contract) rather than a place the backends can drift. `F-Desugar` is the DEFAULT
lowering; where the template's expected type is a **target** (`F-Target` below) the same syntax
lowers to method calls instead, and the template's value is that type rather than `text`.

### Targets — the same template builds a VALUE instead of text (@PLN124)

```
  (F-Target)     when a template is CHECKED AGAINST a type τ that opts in, it does NOT build text:
                 it lowers to a fresh τ and, in source order, calls  τ.lit(s)  for each literal run
                 and  τ.hole_<kind>(v)  for each interpolation.  The template's value is the τ.
                 A type opts in by DEFINING those methods (structural, like an interface — there is
                 no annotation).  The kind is DERIVED from the hole's type, never chosen:
                   text/text? → hole_text · integer → hole_int · float → hole_float ·
                   single → hole_single · boolean → hole_boolean · character → hole_character ·
                   a struct/enum → hole_<type name in method case> (`SqlIdent` → hole_sql_ident,
                   an acronym run breaking at the last capital: `SQLIdent` → hole_sql_ident).
  (F-Target-Pos) the expected type is taken from an ANNOTATED binding (`q: Query = "…"`) or a
                 struct-literal FIELD of the target type.  A call ARGUMENT and a RETURN position do
                 NOT target (measured: `expected Query, got text`); route through a local.
  (F-Target-Kind) a hole whose kind the target does not define is a STATIC error — "Query has no
                 `fn hole_int(self: Query, v: integer)` — declare one to accept this hole".
  (F-Target-Spec) a format SPEC on a hole of a target template is a STATIC error — "a format spec
                 has no meaning on a Query hole — the value is handed to the type, not rendered".
                 F-Spec applies to text templates only.
```

**In words.** The point is that a value which has been rendered into text can no longer be told
apart from text the author wrote. A target keeps them separate: the type sees the author's bytes
through `lit` and each interpolated value through a `hole_…` method, so an interpolated value has
no route into the syntax — which is what lets a library build a SQL statement, a shell command or
a path in which a value can never *become* syntax. Because the method name is derived from the
hole's type rather than chosen, a target and the parser cannot disagree about what a hole is
called, and the diagnostic names the exact method to add. There is no new syntax: the same
`"…{e}…"` either builds text or builds a `Query`, decided entirely by the type it is checked
against.

### Rendering per type — the default form, and null

```
  (F-Render)  ⟦v⟧ (no spec) renders by the value's type:
                integer / long      decimal digits            null (i64::MIN)  → "null"
                float / single      shortest round-trip form  null (NaN)       → "null"
                boolean             "true" / "false"          null (0xFF)      → "null"
                text                the characters as-is      null (STRING_NULL) → "null"
                character           the single codepoint      null (0)         → ""  (nothing)
                enum                the variant name          null (0xFF disc) → "null"
                vector              "[e₀,e₁,…]"  (compact, elements rendered by F-Render)
                struct              "{field:value,…}"  (compact loft form; the `:j` spec → JSON)
```

**In words.** Each type has one canonical text form. A **null** of any type renders as the literal
word `null` — the single exception is a null **character** (codepoint 0), which renders as nothing
(so iterating text past its end appends no garbage). A vector is a compact bracketed list; a struct
is a compact `{field:value}` form, and the `:j` spec switches it to JSON with quoted keys (verified:
`{r:128,g:0,b:128}` vs `{"r":128,"g":0,"b":128}`).

> **A TUPLE has no row here, and its absence is a DECISION rather than a gap.**
> `"{t}"` on a `(integer, integer)` is `error: Cannot format type (integer, integer)`, and
> [TUPLES.md § Non-goals](../TUPLES.md) declares whole-tuple formatting a compile error, with
> the cure named: a named struct, or element-by-element access. The row is stated as absent
> because a reader checking `(F-Render)` against the compiler finds a refusal the rules do not
> mention and reads it as an unruled edge — which is a report, not a defect. Its neighbour
> `(T-Absent)` gives the reason a tuple is the type that keeps meeting this: *"a tuple has no
> faithful document form anyway"*, which is the same argument one level down. A tuple's
> MEMBERS render by their own rows, so `"{t.0}"` is the supported spelling.

### Format spec — width, alignment, padding, precision, radix, sign

```
  (F-Spec)   after `:` a spec tunes ⟦v⟧_f:
               width N            minimum field width (numbers right-, text left-aligned by default)
               < > ^              force left / right / centre alignment
               0N                 zero-pad a number to width N
               .P                 fixed P fractional digits (float/single)
               x  X  b  o  d     integer radix: hex / HEX / binary / octal / decimal
               #                  prefix the radix marker (`0x`, `0b`, `0o`)
               +                  always show a sign on a number — integer, float and single
                                  alike; a negative number keeps its own sign, and `null`
                                  (a sentinel, not a number) takes none
             width counts Unicode CODEPOINTS, not bytes, and a width at or below zero
             asks for no padding — the renderers reach one by subtracting a sign or a
             radix marker they emitted first.
             The flags may be written in any ORDER (`+<8.3` and `<+8.3` are the same spec).
  (F-Spec-Zero)  the `0N` pad fills the NUMBER, so anything the renderer emitted ahead of
                 the digits stays in front of the zeros: `{-1:04}` is `-001` and
                 `{255:#06x}` is `0x00ff`.  Every OTHER pad character shapes the rendering
                 as a whole.  `null` is a sentinel rather than a number and so takes NO
                 zero pad on any type (`{n:08}` is four spaces and `null`), by the same
                 line F-Spec draws for the sign; a non-zero fill still applies to it.
  (F-Spec-Radix) which radixes a hole admits is decided by the renderer its TYPE has:
                 integer/long take `x X b o d` (and `#`), a vector/struct/enum takes the
                 JSON switch `j`, and every other type renders exactly one way and so takes
                 the decimal default alone.  A `.P` precision is admitted by `float` and
                 `single` only — they are the types that HAVE fractional digits.
  (F-Spec-Fill)  a spec may open with a FILL character, which pads instead of a space
                 (`{s:*>6}` is `**​**ab`).  It comes FIRST, before the flags, and it is a
                 single token that is not itself a flag — so a DIGIT can never be one, and
                 `0>5` is not fill-`0` right-align-5 but the comparison `0 > 5`.  `0N`
                 (F-Spec above) is how a number is zero-padded.
  (F-Spec-Exec)  every part of a spec reaches the renderer for the hole's type, or the
                 program is REFUSED.  A part that type cannot execute — a radix or a
                 precision outside the sets F-Spec-Radix gives it — is a static error
                 naming the part; it is never dropped, because a dropped part is a wrong
                 rendering that nothing reports.  The refusal is stated per RENDERER and
                 not as a list of types, because a list is the shape that cannot say what
                 it left out.
```

**In words.** `{1:03}` is `001`, `{42:#x}` is `0x2a`, `{334.1:.2}` is `334.10`, `{"abc":>7}` is
`    abc`. Numbers pad on the left (right-aligned) and text pads on the right (left-aligned) unless
an explicit `<`/`>`/`^` overrides it; the width is measured in characters, so a multi-byte glyph
still counts as one column.

**A spec tunes ⟦v⟧, so it composes with `F-Render` for EVERY type — there is no type whose
rendering a width cannot pad.** `{c:>5}` on a character, `{v:>12}` on a vector and `{p:*^16}` on a
struct all pad the form `F-Render` gives them, exactly as `{"abc":>7}` pads text. Field-shaping
(width, `<`/`>`/`^`, the pad token) applies to the rendered result; the flags that choose the
RENDERING itself (`#`, the `:j` radix) belong to `F-Render` and are not field-shaping.

That composition decides the one edge worth stating: a **null character renders as nothing**
(`F-Render`), so `{nc:>3}` is three pad characters — a full field of them, not an empty string.
Nothing is still a rendering, and a width pads whatever the rendering is.

### Fault-safety — an uncomputable inside `{…}` renders a tagged null, never halts

```
  (F-FaultSafe)  a fault-prone operation inside an interpolation (÷0, index OOB, a field of null)
                 follows operational.md E-Uncomp: it yields the null VALUE, and the interpolation
                 renders it as "null" annotated with the fault cause, e.g. "null(/0)".  Formatting
                 a value NEVER traps or halts — the template always produces text.
```

**In words.** `"{a / b}"` with `b = 0` renders `null(/0)` rather than crashing the program — the
formatter rewrites fault-prone operations in interpolation position to their nullable peers (C66 /
@P376), so building a diagnostic string can never itself fault. The tag (`/0`, an out-of-range
index, …) names *why* the value is null, which is exactly what a `"{x}"` in a log line wants.

---

## Deviations

**OPEN: 0.**

The full register — every entry, open and closed, with its dates and issue numbers — is
the companion [formatting-history.md](formatting-history.md).

## Conformance

- **Interpolation + escape (`F-Interp` / `F-Escape`)** — `"{{lit}}"` is `{lit}`; `"{40 + 2}"` is
  `42`; `"{col}"` renders the struct.
- **Per-type render + null (`F-Render`)** — `"{true}"` is `true`; `"{[1,2,3]}"` is `[1,2,3]`;
  `"{col}"` is `{r:128,g:0,b:128}` and `"{col:j}"` is `{"r":128,"g":0,"b":128}`; `null as integer?`
  renders `null`.
- **Format spec (`F-Spec`)** — `"{1:03}"` is `001`, `"{42:#x}"` is `0x2a`, `"{334.1:.2}"` is
  `334.10`, `"{\"abc\":>7}"` is `    abc`, `"{0.5:+.3}"` is `+0.500`.  **A differential
  oracle cannot see a flag both backends drop** — `+` was honoured on an integer and
  ignored on a float for as long as the suites had no cell for it, and the two backends
  agreed throughout (loft#1087).  The `+`-on-a-float cells are in
  `tests/scripts/14-formatting.loft` for that reason: what pins a rule is a cell that
  spells out the expected string, not the agreement of two implementations.
- **Fault-safety (`F-FaultSafe`)** — `a = 5; b = 0; "{a / b}"` is `null(/0)` on both backends, and
  the program continues.
- **Target (`F-Target`)** — with `lit` + `hole_text` + `hole_int` on `Query`,
  `q: Query = "SELECT * FROM t WHERE name = {name} AND id = {n}"` leaves `len(q.parts) == 2` and
  `q.values == ["ada", "7"]` — identical on both backends; the same template assigned to `text`
  renders the ordinary string. The two refusals (`F-Target-Kind`, `F-Target-Spec`) reject on
  `--dump` / `--interpret` / `--native` alike (the D-op-2 driver-agreement facet).

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on a rendered
string is the definitional error this doc names.
