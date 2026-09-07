<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/types.md — type system & conversion relation (strict)

**Catalogue:** @F3 (scalar types), @F4 (width integers), @F5 (type conversions), @F1 (null / Optional). Roadmap: @PLN88, @PLN25.

> **Rules then deviations** (see [README](README.md)). The rules are the target loft's
> front end should satisfy; the deviations are where today's code breaks them, to be
> driven to zero. Analysis/rationale lives in [../TYPING_RELATION.md](../TYPING_RELATION.md)
> (the lens) — entries here link to it instead of re-explaining.

## Notation

- `Γ` — typing context (variable ⟼ type bindings).
- `τ, σ` — types (the `Type` enum, `src/data.rs`).
- `&τ` — a **reference type**: the type of a variable that is a live link to a τ-lvalue
  (read/write-through). A type constructor; its introduction and link semantics live in
  [binding.md](binding.md). In *this* doc it appears only in the conversion relation —
  it reads through to its referent (`C-Ref`).
- `Integer[a,b]` — an integer type with closed value range `[a, b]` (the `IntegerSpec`
  min/max). `integer` = `Integer[i64::MIN+1, i64::MAX]` (symmetric — the reserved null
  sentinel `i64::MIN` is EXCLUDED from the value range; see
  [operational.md `(E-Null)`](operational.md)); `u8` = `Integer[0, 255]` (a *non-null* `u8`;
  a nullable `u8?` reserves `255` for null, so its non-null values are `[0, 254]`); etc.
- `τ ⤳ σ` — **conversion**: a value of `τ` is accepted where `σ` is expected, with no
  explicit cast. This is the *only* implicit coercion in the language.
- `⊔` — the **join** (least type containing both); for integers, the range-union's
  enclosing `Integer[min a c, max b d]`.

---

## Rules

### Judgments — bidirectional, two modes only

```
  (T-Syn)   Γ ⊢ e ⇒ τ        e synthesises its own type τ
  (T-Chk)   Γ ⊢ e ⇐ τ        e is checked against an expected τ
  (T-Sub)   Γ ⊢ e ⇒ τ ,  τ ⤳ σ   ⟹   Γ ⊢ e ⇐ σ
```

**In words.** loft works out a type in one of two directions: either the expression tells
us its type (`⇒`, *synthesis*), or the surrounding code already expects a type and we
check the expression against it (`⇐`, *checking*). `(T-Chk)` is the **single** carrier of
"the expected type" — there is exactly one checking mode, pushed structurally into
sub-expressions (literals, lambda bodies, variant references, read targets). There is no
other channel for an expected type.

```
  (T-Chk-Vec)    Γ ⊢ [e₁ … eₙ] ⇐ vector<τ>   ⟸   ∀i. Γ ⊢ eᵢ ⇐ τ
  (T-Chk-Lam)    Γ ⊢ (\x.e) ⇐ fn(σ)→τ         ⟸   Γ, x:σ ⊢ e ⇐ τ
  (T-Chk-Var)    Γ ⊢ V ⇐ Enum E               ⟸   V ∈ variants(E)
```

See **§ Nullability** below for `τ?` (the optional former) and the rules that introduce /
eliminate it; indexing (`(N-Index)`) is one of the fallible operations that synthesise it.

### Conversion `τ ⤳ σ` — width folded in

```
  (C-Refl)    τ ⤳ τ
  (C-Never)   Never ⤳ τ
  (C-Tuple)   (σ₁…σₙ) ⤳ (τ₁…τₙ)        ⟸   ∀i. σᵢ ⤳ τᵢ
  (C-Var)     Reference(S) ⤳ Enum(E)   ⟸   S ∈ variants(E)            (and plain
              Enum ⤳ Integer tag).  NB: the null INTRO `S ⤳ S?` is `(N-Intro)`; there is
              NO implicit `S? ⤳ S` unwrap (that is `(N-Store)`-illegal — discharge via `??`)
  (C-Int)     Integer[a,b] ⤳ Integer[c,d]   ⟸   [a,b] ⊆ [c,d]         (see I-*)
  (C-Ref)     &τ ⤳ σ   ⟸   τ ⤳ σ      (a reference reads through to its referent; there
              is NO  σ ⤳ &τ  — a reference is made only by `&` at a binding, never coerced)
```

**In words.** `τ ⤳ σ` is loft's *only* automatic conversion — "a `τ` value is fine where a
`σ` is wanted, no cast needed." The rules list the safe cases: the same type; a `Never`
(a `return`/`break`, which fits anywhere); tuples element-by-element; a struct used as one
of an enum's variants (and the nullable/tag duals); and an integer into a *wider* integer.
`(C-Int)` means **width lives inside `⤳`**: an integer flows into another integer iff its
range fits. There is no separate width authority. `is_equal` answers only the *width-free*
base-type question ("is this `integer`?" — correctly *yes* for every width); `convert` and
codegen read width from this one relation, never their own copy (see the integer model
below). `(C-Ref)` threads in
references: a `&τ` variable *has* type `&τ` and reads **through** to a `τ`, so it is
usable wherever a `τ` is, and `τ`'s own conversions then apply (e.g. `&u8` → `integer`).
The reverse never holds — you cannot coerce a plain value into a reference; a `&τ` is made
only by a `&` annotation at a binding. The link's introduction and its write-through
semantics live in [binding.md](binding.md); here it is just one more thing `⤳` accepts.

### Nullability — the optional former `τ?`

> **One former, representation derived — the integer model applied to null.** `τ?` is the
> *optional* type ("a `τ`, or `null`"). **Storage is non-null by default**: a binding,
> field, or `vector` element of type `τ` never holds `null` — `τ?` is the *only* way a slot
> admits it. `not null` still parses (kept for source back-compat) but is **deprecated**: it
> is a semantic no-op (non-null is already the default) and, since #546, WARNS ("`not null` is
> deprecated and has no effect… delete `not null`") rather than staying silent. Null enters
> values only through the **fallible operations**
> below, which synthesise `τ?`; it leaves only through an explicit **discharge** (`??` /
> `match`). There is no implicit unwrap.

```
  formation
  (N-Opt)      τ wf  ⟹  τ? wf          τ? is a type for any τ
  (N-Idem)     τ?? ≡ τ?                 optional is idempotent — no double-null
  (N-Dense)    vector<τ> stores τ       elements are non-null unless written vector<τ?>

  introduction (a non-null value flows into an optional slot)
  (N-Intro)    τ ⤳ τ?                   the ONLY null-direction conversion in ⤳

  "no representable result" → τ?.  null is the UNIVERSAL doesn't-fit / undefined value —
  NEVER wrap, saturate, or an out-of-range value (that would be UB).  Nullability is
  RANGE-DRIVEN: an op is non-null when its result provably fits, τ? when it could miss.
  (N-Index)    Γ ⊢ v ⇒ vector<τ>, Γ ⊢ i ⇒ Integer   ⟹   Γ ⊢ v[i]  ⇒ τ?       (OOB — no elem)
               EXCEPT an index trusted BY CONTRACT — a constant, an active `for` variable, a
               read guarded by `i < len(v)`, integer arithmetic over those (@PLN102 D1,
               `index_provably_fit`) — which reads τ NON-NULL; a genuine overrun there
               faults to null at run time (C80) and no rule above the runtime sees it.  So a
               nullability cell built on `v[9]` measures nothing: it must index by a plain
               variable.
  (N-Div)      Γ ⊢ a,b ⇒ Integer                     ⟹   Γ ⊢ a/b, a%b ⇒ Integer?  (÷0 undefined)
  (N-Parse)    Γ ⊢ parse_τ(s)                          ⟹   Γ ⊢ …     ⇒ τ?           (invalid)
  (N-Arith)    Γ ⊢ a,b ⇒ Integer,  op ∈ {+,-,*}      ⟹   Γ ⊢ a op b ⇒ Integer[r]
               where r = the range-arithmetic of the operands.  NON-null if r ⊆ i64 (the
               common bounded case — no `??`); else Integer? (overflow → null).
  (N-Cast)     Γ ⊢ e ⇒ Integer[s]   ⟹   Γ ⊢ (e as τ) ⇒ τ   REQUIRES s ⊆ range(τ).  If the
               fit is NOT provable it is a COMPILE ERROR — `as τ` is an assertion of fit; the
               honest form for a maybe-miss is `as τ?`.  (`b: integer; b as u8` errors; `400
               as u8` errors — provably can't fit.)
  (N-Cast?)    Γ ⊢ (e as τ?) ⇒ τ?    the CHECKED cast — value if it fits, else null.  Always
               legal; NEVER yields an out-of-range value.  (`b as u8? ⇒ u8?`.)

  elimination (discharge — REQUIRED; there is NO  τ? ⤳ τ)
  (N-Coal)     Γ ⊢ e ⇒ τ?,  Γ ⊢ d ⇐ τ                ⟹   Γ ⊢ (e ?? d) ⇒ τ
  (N-Default)  Γ ⊢ e ⇒ τ?,  has_default(τ)           ⟹   Γ ⊢ e? ⇒ τ            (@PLN116)
               the TYPE's default discharges:  e?  ≡  e ?? construct_default(τ).
               `has_default(τ)` is a STATIC side-condition — where it fails, `e?` is a
               COMPILE error, never a runtime one (§ Defaults below).  The pairing is the
               mnemonic: `??` = the default YOU give, `?` = the default the TYPE gives.
  (N-Match)    match e { null ⇒ …,  x ⇒ …(x:τ)… }      eliminates τ?, binds the τ arm
  (N-Store)    storing  e:τ?  into a  τ  slot without discharge is REJECTED — a WARNING for
               most τ (the null is representable-and-distinct in τ's non-null form), a hard
               ERROR only for narrow widths (u8…u32, § Null-flow below, where the null would
               collide with a real value); discharge first (`?? d` / `match`) either way.
               A SLOT is every position that holds a τ: a local, a field, a collection
               element, a tuple member, a call argument, a return, an INDEX (`v[i]`,
               `s[a..b]` — a null index reads null).  Reading a τ? is not storing it: a
               comparison, a null test, a condition, `??`'s subject, `match`, a
               null-transparent callee and an `if` arm meeting its sibling's type
               (`(N-Join)`) say nothing.

  inference — declared vs inferred storage (the "by definition vs by use" split)
  (N-Decl)     a DECLARED slot `x: τ` is a COMMITMENT: `x = e` checks `e ⇐ τ`. If e:τ? it is
               `(N-Store)`-illegal — declaring non-null FORBIDS a later nullable write.
  (N-Join)     an INFERRED `a = e₁ … a = eₙ` (no annotation) has type `⨆ᵢ τᵢ`, made OPTIONAL
               iff some `τᵢ` is optional.  `?` rides the SAME join as integer width `(I-Join)`
               — `integer ⊔ integer? = integer?`.

  @PLN102 (SHIPPED, default-on since 2026-07-11 — see § Null-flow, the general laws below):
   · (N-Prop)   null PROPAGATES through arithmetic (a:τ? op b ⇒ τ?), across every type.
   · (N-Parse)  FOLDS INTO (N-Cast): a parse is a cast — `s as τ` asserts (non-null), `s as
                τ?` checks.  The auto-`τ?` reading above is superseded.
   · (N-Store)  becomes a WARNING (not ill-typed) EXCEPT narrow widths (u8…u32), where the
                null collides with a real value and it stays a hard error.
```

> **The case that proves the declaration means something** (`do we allow a:integer=2; a=v[i]`?):
>
> | code | result | why |
> |---|---|---|
> | `a: integer = 2;  a = v[i]` | **warning**, `a` stays `integer` and holds the null | `a` declared non-null `(N-Decl)`; `v[i]:integer?` is `(N-Store)`'s store — a WARNING at full width (an ERROR for `a: u8`) — write `a = v[i] ?? 0` or declare `a: integer?` |
> | `a = 2;  a = v[i]`          | `a : integer?` | inferred → `(N-Join)` widens to optional |
>
> Exactly parallel to declared integer width (`a: u8 = 2; a = big` also errors — a declared
> type is a commitment; an inferred one widens by join).  The declared row's severity is
> `(N-Store)`'s own, not a stricter one: the declared local was the one slot the compiler
> refused at every width while every other slot warned (`D-Decl-Sev`, closed 2026-09-05).

**In words.** Nullability is a property of **operations, not storage**. `null` is the
**one universal value for "no representable result"** — division by zero, out-of-bounds
index, failed parse, integer overflow, and a cast that doesn't fit **all yield `null`**,
never a wrapped / saturated / out-of-range value. That single choice is what **roots out
the UB class**: a slot of type `τ` never holds a non-`τ` value — it either fits (and the
op is non-null) or it's `null` (and `(N-Store)` forces you to discharge it). We do **not**
fake non-null on an op that can miss.

> **The one decided exception — overflow arithmetic ([C85](../DESIGN_DECISIONS.md#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--)).**
> `a+b` / `a*b` / `a-b` stay typed **non-null `integer`** (forcing `integer?` on every
> arithmetic op would poison the common path to guard a fault that essentially never fires),
> yet on overflow they write the reserved `i64::MIN` sentinel into that non-null slot — which
> then reads as `null`. So a non-null `integer` slot *can* observably hold `null` after an
> overflow: `(N-Store)` never fires because the op is typed non-null. This is a **deliberate,
> bounded soundness edge** (parallel to a non-null `float` holding a `NaN`), not a deviation to
> close — the reachable-fault ops (`/`, `%`, `v[i]`, parse) stay `τ?` as above.
>
> **A sentinel has to EXIST for that to be writable, and for the width aliases it sometimes
> does not.** The rule reaches exactly the specs that keep a code back for null and put it at
> the BOTTOM, which is the code a non-null read already reports as null:
>
> | spec | spare code? | overflow answers |
> |---|---|---|
> | `integer` (i64), `i32` | yes, and it is the bottom (`i32` is `i32::MIN + 1 ..= i32::MAX`, which is why `i32 = -2147483648` is refused) | `null` |
> | `u32` | yes, but at the TOP — no non-null read tests for it, and writing it renders `4294967295`, outside the type's own range | the type's default |
> | `u8`, `i8`, `u16`, `i16` | no: the range FILLS the width, so all 256 / 65 536 codes are values | the type's default |
>
> The default is `(E-Uncomp-NN)`'s and is at least a legal value of the type, so nothing holds
> a non-`τ` value — but it is indistinguishable from a computed one, which is the price of a
> width with no code to spare. `τ?` is the spelling that always answers `null`, because a
> nullable narrow alias sacrifices an edge value to reserve one (`(N-Reserve)`). loft#1296.
>
> **And the answer does not depend on where the slot lives.** The collapse site
> (`OpRangeDefault`, shared by both backends) exempted the sentinel unconditionally, so a
> non-null narrow LOCAL — an `i64` until it is materialised — held it and read back `null`,
> while the same overflow reaching a FIELD or an ELEMENT narrowed to the default. One
> declared type, two answers, decided by whether the arithmetic landed exactly on the
> sentinel. The guard now reads the target's nullability off `dflt`, which
> `uncomputable_default` already sets to `i64::MIN` exactly when the slot can read back null,
> so a sentinel is passed through where null is representable and collapsed where it is not
> (loft#1305). The RETURN divergence closed with it — the interpreter kept the sentinel in an
> 8-byte slot and native emitted `as u8` — because the sentinel no longer survives the store
> (loft#1306).

Nullability is **range-driven**, so the discharge burden is proportional to *real* risk,
not theoretical: `a op b` and `e as τ` are **non-null when the result provably fits** the
target range — `b=4; b*100 ⇒ Integer[400]` fits i64, so it's non-null, **no `??`** — and
`τ?` only when the range could miss (a narrowing `as`, a declared-narrow slot, a genuinely
i64-overflowing product). So there is **no `??` "after every `a*x`"** — only where the op
can actually fail to produce an in-range value. `(N-Intro)` is the one implicit
null-direction step; the reverse is never implicit, so a null can't be lost by accident.

This unifies nullability with the **integer range model into one system**: a value's range
decides its storage width *and* whether a fit-failure yields `null`. Overflow-to-null is
therefore the *correct* runtime behavior (loft already does it for `a*b`); the work is to
**type** it (`Integer?` when the range exceeds i64) and require discharge — and to fix
`as` so `400 as u8` yields `null`, not the current `400`.

**Representation is derived, exactly like integer width.** `τ?` has one identity
(optional-of-τ); how the `null` is *stored* follows from the base type and is not part of
the type:

| base `τ` | `τ?` null representation (the reserved value, per width) |
|---|---|
| `Integer` (8-byte) | in-band sentinel `i64::MIN` |
| a narrow `Integer` (`u8`/`i8`/`u16`/`i16`/`i32`) | in-band sentinel = its top stored value (`u8` → `255`); excluded from `τ?`'s non-null range |
| `Bool` | in-band sentinel `255` |
| `Char` | in-band sentinel codepoint `0` (collides with a literal `'\0'`) |

> **The sentinel is the type's, not each reader's — measured 2026-08-22.** loft#1014 made
> every site that WRITES a character agree on codepoint 0; five sites that READ one, or put
> a character on the wire, still spelled it their own way, and `Stores::is_null`'s character
> arm was unreachable as well as wrong. An absent character therefore serialised as a SPACE,
> and `to_json()` emitted the loft literal `'q'` — not JSON — so a struct with a character
> field could not round-trip through loft's own parser. On the wire a character is now a
> one-character JSON **string**; a number is still read as its codepoint. Guarded by
> `tests/scripts/character-across-the-json-surface.loft` on both backends, and the reason
> the interpolation channel still differs (concatenation skips a bare `'\0'`) is the
> collision this row names. QUALITY.md § `character` on the JSON surface.
| `Float` / `Single` | in-band sentinel = a reserved `NaN` |
| a reference | out-of-band `nullref` (a reserved `DbRef`; no collision) |
| an `enum` (plain or struct-enum) | in-band **discriminant `0`** — variants are numbered from 1, so `0` is a variant of no enum; a plain enum reads `255` as absent too (the byte an explicit `null` writes), which is why it holds at most 254 variants. No collision, and `size(E?) = size(E)` |
| a struct `S` as a `vector` element | the tagged **`__nullable<S>`** enum (discriminant + payload; no collision) |

> The in-band scalar sentinels are **observable, reserved values** — the base type's null is
> one specific bit-pattern, excluded from the non-null range (`(E-Null)`). This is the
> deliberate zero-overhead choice ([the null-model keystone](../plans/102-stability-contract/keystone-null-model.md),
> option B); its cost — a nullable narrow type cannot store its one reserved value — is a
> documented limitation, and the collision sites that could silently *produce* a sentinel are
> guarded (D-op-null-2). References and struct-in-vector already avoid the cost with an
> out-of-band tag.

So `bytes_for(τ?)` and the sentinel-vs-tag choice are *consequences* of `τ`, never declared
— the same discipline as `bytes_for_range` on integers. **Parametricity holds**: `τ?` and
`vector<τ>` formation commute with substitution (`⟦N?⟧[N:=S]=⟦S?⟧`,
`⟦vector<N>⟧[N:=S]=⟦vector<S>⟧`), because nothing is rewritten on the element's *shape*.

(Design + the evidence that the old implicit default-nullable rewrite broke parametricity
and forced materialisation: [../plans/25-nullable-sequences/storage-vs-access-nullability.md](../plans/25-nullable-sequences/storage-vs-access-nullability.md).)

### Defaults — `construct_default` and the `x?` discharge (@PLN116)

`(N-Default)` discharges a `τ?` with **the type's own** default. That default is ONE partial
function on types, shared with the `S{}` zero value (one home per fact,
[Goal E](../GOALS.md)) — so `x?` and `S{}` can never disagree about what a `τ` defaults to.

```
  construct_default : τ ⇀ value        PARTIAL — has_default(τ) is exactly its domain

  (D-Scalar)   construct_default(Integer[r]) = 0            (every width)
               construct_default(Float)   = 0.0
               construct_default(Single)  = 0.0f
               construct_default(Boolean) = false
               construct_default(Character) = '\0'
  (D-Text)     construct_default(text)      = ""
  (D-Coll)     construct_default(vector<τ>) = []            (likewise every keyed collection)
  (D-Opt)      construct_default(τ?)        = null          an optional's default IS null
  (D-Enum)     construct_default(Enum E)    = the FIRST-DECLARED variant of E
  (D-Rec)      construct_default(S)         = S{f₁ = d₁ … fₙ = dₙ}
                 where dᵢ = the field's `= expr` when given, else construct_default(τᵢ)
                 REQUIRES  ∀i. has_default_field(fᵢ)

  the two places the domain STOPS — has_default = FALSE, so `x?` (and `S{}`) is a COMPILE error
  (D-NoRef)    has_default(&τ) = FALSE
                 a bare reference / non-null DbRef has no zero: there is no "the null pointer"
                 in a language whose storage is non-null by default.
  (D-NoEnumF)  has_default_field(f : E) = FALSE  when E is a BARE (non-optional) enum and f
               carries no `= expr`.
                 An enum's 0 IS its null (variants are 1-based), so a non-null enum field may
                 not silently zero-fill to it; and choosing a variant as a record's default is
                 a real decision the author must make.  Fix by supplying the field, giving it
                 `= <variant>`, or typing it `E?` — which then defaults to `null` by (D-Opt).
```

**In words.** Every type either has one obvious zero or it has none, and `x?` is exactly
"discharge with that zero". Scalars go to `0`/`false`/`'\0'`, `text` to `""`, collections to
empty, a record to itself with every field defaulted. Two things genuinely have no zero, and
for those `x?` does not compile — you must say what you mean with `??` or `match`.

**The partiality is a TYPE rule, not a runtime one.** This is what keeps `x?` consistent with
"no *runtime* errors, ever" ([C80](../DESIGN_DECISIONS.md)): `x?` never fails at run time
because the cases where no default exists are rejected at *compile* time. `has_default` is a
static well-definedness condition on the operator, in the same family as `(N-Cast)`'s
provable-fit requirement — not a check that can fire on a value.

**A bare enum discharges positionally — say it out loud.** `(D-Enum)` makes `x?` on an enum
mean *the first-declared variant*, so **reordering the variants silently changes what `x?`
does**, while `x ?? Colour.Red` is order-independent. That asymmetry is deliberate (a bare
enum has to default to *something*, and first-declared is the only choice that needs no extra
declaration), but it means `?` on an enum trades an explicit choice for a positional one.
Prefer `??` where the variant matters. Note the contrast with `(D-NoEnumF)`: a *bare* enum
discharges to its first variant, yet an enum *field inside a record* refuses to — because
defaulting a whole record silently is a much bigger claim than defaulting one expression.

**At a text parse, `?` composes with the CHECKED cast, not the asserting one.** A bare
`s as integer` on text is ill-typed under `(N-Cast)` — a parse cannot be *asserted* — so
`(s as integer)?` is rejected by the inner cast, before `(N-Default)` is ever reached; its
premise `Γ ⊢ e ⇒ τ?` simply does not hold. The composable form is the checked cast first:

```loft
(s as integer?)?          // integer   — checked cast ⇒ integer?, then `?` ⇒ 0 on a bad parse
s as integer ?? 0         // integer   — the assert-or-default form `(N-Cast)` licenses
s as integer              // COMPILE ERROR — an assertion a parse can't discharge
```

This is not a gap in `(N-Default)`: `?` discharges the parse result exactly like any other
`τ?`. Only the *asserting* spelling is refused, and it is refused for reasons that have
nothing to do with `?`.

**Falsifying programs.**

```loft
// (D-Enum) — obeying the rule and reordering the enum disagree
enum Colour { Red, Green, Blue }
c: Colour? = null;
c?                        // Red.  Swap Red/Green in the declaration and this becomes Green.

// (D-NoEnumF) — a record whose enum field has no default cannot itself default
struct Pixel { tint: Colour, x: integer }
p: Pixel? = null;
p?                        // COMPILE ERROR: `tint` is a bare enum with no `= expr`
                          // fixes: `tint: Colour = Colour.Red`, or `tint: Colour?`

// (D-Opt) — an optional defaults to null, so `?` on it is a no-op, not an unwrap
o: integer? = null;
o?                        // 0        (τ = integer here — `?` discharges the outer optional)

// (D-Rec) — the default is structural, not a shared instance (see operational.md E-Default)
struct P { x: integer, y: integer }
q: P? = null;
q?                        // P{x:0, y:0}
```

### Null-flow — the general laws, across EVERY type (@PLN102, 2026-07-11)

The `(N-*)` rules are stated on `τ` and hold for **every** type, not just `integer`. The null
model is ONE model: each type reserves exactly one null ([C90](../DESIGN_DECISIONS.md); table
below), and four general laws govern how a null is *produced*, *propagated*, *asserted away*,
and *stored*. They are checked **throughout the stack** by a cross-type conformance matrix
(each type × each law, both backends), so a gap in any one type is a caught deviation, not a
silent hole.

**Shipped (2026-07-11, default-on — @PLN102 #559; `nullflow_enabled()` in `src/keys.rs`, opt-out
`LOFT_NO_NULLFLOW`):** (N-Prop), the (N-Store) warn/error split, the (N-Domain) generalisation,
and folding (N-Parse) into (N-Cast) are live across every type, verified both backends. Float
`/`/`%` and the domain-partial functions (`sqrt`, `ln`, `log`, `asin`, `acos`, `pow`) now type
`τ?` exactly like integer `/`/`%`: a variable-divisor `1.0 / b` stored into a non-null `float`
WARNS (the (N-Store) split above, not a uniform hard error); and a text parse `s as float` is
now a hard COMPILE ERROR ("a text parse `as float` may fail — use `float?`"), superseding the
old auto-`τ?` reading. Design record:
[../plans/102-stability-contract/float-null-domain-typing.md](../plans/102-stability-contract/float-null-domain-typing.md).

```
(N-Domain)  a PARTIAL OPERATION that can yield the reserved null from a REACHABLE input types
            its result τ?  —  ÷0 / %0 (Integer / Float / Single); sqrt(<0), ln·log(≤0),
            asin·acos(∉[-1,1]), pow(neg ^ frac) (Float / Single); v[i] / s[i] OOB (ANY element
            τ).  Non-null when the input is PROVABLY in-domain (constant / range / guard) — the
            same "provably-fits" elision as (N-Arith) / (N-Cast).  Generalises (N-Div) /
            (N-Index) to every type + partial op.  Runtime = null + continue (C80); the reserved
            null is the VALUE, NEVER a runtime error.

(N-Prop)    an operation with a NULLABLE operand whose runtime carries the null through types
            its result nullable:  a:τ? op b  ⟹  τ?  (either operand).  Arithmetic on
            Integer / Float / Single (the sentinel / NaN propagates), text `?`-concat, etc.
            The type tracks the propagation the runtime ALREADY performs (verified: `n+5`,
            `5-n`, `abs(n)` on a null n stay null).  C85 is the COMPLEMENT — non-null operands
            stay non-null; a sentinel PRODUCED by overflow is a result, not a propagated INPUT.

(N-Cast)    an explicit cast `as τ` is an ASSERTION → non-null τ (compile error if the fit is
            not provable — use `as τ?` / `?? d`).  A text→numeric PARSE is a cast, so it obeys
            (N-Cast) / (N-Cast?): `s as float` asserts (non-null), `s as float?` checks (→
            float?), `s as float ?? d` is assert-or-default.  This SUPERSEDES the old auto-`τ?`
            reading of (N-Parse): the `?` on a cast is the programmer's explicit choice, never
            inferred, for EVERY τ.  (An OPERATION's `?` is inherent (N-Domain); a CAST's is not.)

(N-Store)   storing e:τ? into a non-null τ slot is —
            · a WARNING (nudge, compiles + runs, the slot holds null) when the null is
              REPRESENTABLE-AND-DISTINCT in τ's non-null form: a stored null reads back as null,
              spreadsheet model intact;
            · a hard ERROR when the null sentinel is a VALID non-null value of τ (a collision),
              so the null cannot be stored faithfully — discharge (`?? d`) or widen the slot to
              `τ?`.
            Representability is a property of τ (table).  REFINES the old uniform-error
            (N-Store): every type warns EXCEPT the narrow widths, which error.

(N-Reserve) a reserved null is a VALUE OF THE TYPE, so it is excluded from `τ?`'s non-null
            range — and excluded EVERYWHERE the value can be, not only where the bytes are
            packed.  `255` is a real `u8` and IS the null of a `u8?`, so a `u8?` ranges over
            `0..=254` in a local, a field, an element, a parameter and a return alike.  What
            spends the edge is a SLOT — a place a value is KEPT — and an expression in flight
            is not one: `e as u8?` yields `255` and `(e as u8?) ?? d` keeps it, because
            neither ever holds a `u8?`; assign the same cast into a `u8?` and it is null.
            Which types spend an edge follows from the table above: only a narrow width whose
            range exactly fills a fixed 1- or 2-byte storage — an `i32?` has a spare code
            outside its range and an `integer limit(0,255)?` widens to get one, so neither
            gives anything up.  The COMPLEMENT is the same statement: a NON-null narrow
            reserves nothing, because it has no null to encode.
```

**Per-type null + store verdict** — the verdict follows the *representability* test, not
per-type taste ([C90](../DESIGN_DECISIONS.md) fixes the reserved value):

| τ | reserved null | distinct in the NON-null form? | `τ?` → `τ` store |
|---|---|---|---|
| `integer` (i64) | `i64::MIN` — reserved even non-null (`[MIN+1, MAX]`) | yes | **warn** |
| `float` / `single` | `NaN` | yes (`NaN` ≠ any real) | **warn** |
| `boolean` | `255` (three-state, C73) | yes (non-null = 0 / 1) | **warn** |
| `character` | codepoint `0` / NUL — reserved even non-null (`0 as character` reads null) | yes | **warn** |
| `text` | out-of-band (heap) | yes | **warn** |
| reference | out-of-band `nullref` | yes | **warn** |
| struct in `vector` | tagged `__nullable<S>` | yes | **warn** |
| narrow `u8`/`i8`/`u16`/`i16`/`i32`/`u32` | top width value — reserved ONLY in the `τ?` form | **no** — non-null uses the full width (`255` is a real `u8`) | **error** |

The narrow widths are the **sole error case**: they are the only types whose non-null form
spends the whole width on real values (C90 gives them a sentinel only in `τ?`, to keep the
range full), so a null cannot sit in a non-null narrow slot. Every other type reserves its
null distinctly even non-null — the C85 in-band-sentinel property — so a stored null is
observable and a warning suffices. (Composite `τ?` — tuples, multi-field aggregates — inherits
its elements' out-of-band tags and warns; per-element vs whole-tuple nullability is an open
gap, see [../plans/102-stability-contract/formal-audit.md](../plans/102-stability-contract/formal-audit.md).)

### The integer model — one type; the rest is notation

> **`integer` is the only integer type.** Its identity is its value range `[min, max]`
> (plus `not_null`). `u8` / `i8` / `u16` / `i16` / `i32` are **not** distinct types — they
> are **notation** for an `integer` whose range fits in fewer than 8 bytes. The value
> semantics have no per-width type; a width is a *consequence* of the range.
>
> **Storage width is derived, never declared.** The bytes a value occupies are a pure
> function of its range — `bytes_for_range(min, max)`: `[0,255]` ⟹ 1 byte, the full range
> ⟹ 8. `IntegerSpec.forced_size` is a **cache of that function**, not an independent fact:
> it must always equal `bytes_for_range(min, max)`. It is a storage hint, not a value-type
> (the field's own doc says exactly this).
>
> **Why this settles the width question.** Width has one home — the range, read through
> `⤳` / `(C-Int)`. So `is_equal` collapsing every `Integer(_)` to one type is **correct**
> ("is this `integer`?" carries no width); narrowing (`I-Narrow`) and the codegen cast both
> **derive from the range**, not from `forced_size`. A too-narrow range is then
> unambiguously a bug — undersized storage means a silent overflow, with no second
> authority to mask it — which is *why* range inference (`I-Join`, …) must be sound.
> Narrowing now decides by range containment (`is_narrowing_int`), in agreement with
> codegen's `narrow_int_cast` — so signedness is visible and the old split is closed. The
> one residual (D2) is that the *full integer* is still marked by `forced_size = None`,
> because `IntegerSpec`'s i32/u32 bounds cannot yet hold the i64 range.

### Integer width

```
  (I-Sub)     Integer[a,b] <: Integer[c,d]   ⟺   [a,b] ⊆ [c,d]
              and  <:  is exactly the implicit  ⤳  on integers (C-Int).
  (I-Widen)   widening (a superset target) is implicit.
  (I-Narrow)  narrowing (a non-superset target) is NOT implicit: it needs either
                – an explicit  e as σ , or
                – e is a literal whose value ∈ range(σ), or
                – σ is NULLABLE (σ = τ?), in which case the narrowing is implicit and
                  CHECKED: it is  e as τ?  , yielding the value when it fits and `null`
                  when it does not (I-Narrow-Opt, below).
  (I-Narrow-Opt)  a narrowing into  τ?  is the checked cast, not a refusal.  The reason the
                  first two clauses ask for an `as` is that a narrowing has no defined
                  answer for a value outside the range, and the author is the only one who
                  can supply it — but a NULLABLE target already carries that answer: `null`
                  means "this did not fit", it is visible, and `??` recovers from it.  So
                  the marker would ask for intent the type has already stated.  A
                  NON-nullable narrow target has no such value and is still refused, and
                  the refusal names `τ?` as the cure.
  (I-Lit)     an integer literal n  has every type Integer[a,b] with a ≤ n ≤ b
              (it checks at the expected width; it does not force i64).
  (I-Join)    a variable assigned e₁ … eₙ in a scope has type  ⨆ᵢ τᵢ  where τᵢ are the
              synthesised assignment types.  (Its width is the join of all writes,
              never just the first/narrowest.)
```

**In words.** An integer's type is its value *range*. A narrower integer fits a wider one
for free (`u8` flows into `integer`); the other way round (wider into narrower) needs an
explicit `as`, unless the value is a literal that plainly fits — or unless the target is
NULLABLE, which is the one target that already says what an out-of-range value becomes. A literal takes whatever
width is expected of it. And a variable written in several places gets the type big enough
for *all* its writes (the join) — not just its first one. That last point was the
#433-residual; `(I-Join)` is now implemented for inferred locals (an inferred local widens
to the join when a write would not fit; an annotated `x: u8` stays constrained), guarded by
`tests/scripts/433-ijoin-multiply-assigned.loft`.

**Why `(I-Narrow)`'s `as` is consistent with the maker-centric center.** The explicit `as` is
a *write-time intent marker* at a **rare, deliberate edge** — idiomatic code (a well-built
library used as intended) needs **none**, the bar Rust's `usize`-index `as`-tax breaks by
construction and loft holds by having **one integer type, no special index type**. It is the
*right* moment to ask intent (you are in the editor, choosing to drop bits), not the wrong one
(runtime, or the common path). And it coexists with the spreadsheet model: a narrowing that
overflows *at runtime* yields **null and keeps running** (operational.md `E-Uncomp`, C80) —
`as` is compile-time intent, null is runtime keep-going. See
[GOALS.md § the wrong moment](../GOALS.md) and [DESIGN_DECISIONS.md C79/C80](../DESIGN_DECISIONS.md).

### Coercion closure

```
  (C-Only)    ⤳ is the only implicit coercion.  Every other type change is an explicit
              op or cast and appears in the syntax.
```

### Pattern captures (@PLN35, SHIPPED)

> **@PLN35 · SHIPPED.** These rules were written spec-first, ahead of the code; the code
> landed with phases 1–7 + PC1–PC5 ([matching.md § Rules — PEG patterns](matching.md)) and now
> obeys them — verified both backends: an alternation binding a name in only SOME branches
> reads `null` in the branch that does not bind it (`P-Alt-Diff`), and a capture inside `(a)?`
> reads `null` when the optional is absent (`P-Opt-Ty`). Pinned per-phase by the @PLN89 oracle;
> design: [../plans/35-match-peg/FORMAL-DESIGN.md](../plans/35-match-peg/FORMAL-DESIGN.md).

A pattern capture binds a name to a matched sub-result. **The headline is that this introduces NO
new type former** — every capture lands on `τ`, `τ?`, or `vector<τ>`, unified by the SAME join `⊔`
and nullable-join `(N-Join)` that already run the integer / nullability model above.

```
  (P-Cap-Ty)     Γ ⊢ (name:p)  binds  name : τ         where Γ ⊢ p's result ⇒ τ
  (P-Alt-Same)   (a | b) both bind name : τ_a, τ_b     ⟹   name : τ_a ⊔ τ_b   (the join; if τ_a ⊔ τ_b
                 is undefined ⟹ STATIC ERROR "alternatives bind name at incompatible types")
  (P-Alt-Diff)   name bound in only SOME alternatives  ⟹   name : τ?           (via (N-Join))
  (P-Opt-Ty)     a capture inside (a)?                 ⟹   promoted to τ?       (via (N-Opt))
  (P-Rep-Ty)     the capture inside (a)* / (a)+        ⟹   vector<τ>            (via (N-Dense))
  (P-Rest-Ty)    ..name over a vector<τ> subject      ⟹   name : vector<τ>
```

**In words.** A named capture takes the type of whatever its sub-pattern produced. When two
alternatives bind the same name, the two types are joined (`integer` and `u8` → `integer`; no join
⟹ a compile error, exactly like an incompatible integer join). A name only *some* branches bind, or
a capture inside an optional, becomes nullable (`τ?`) — the same optional-join `(N-Join)` an
inferred `a = null; a = 5` uses. A repetition or `..rest` collects into a `vector<τ>`. So PEG
capture typing is a new *source* of the types loft already has; `match` also stays a `τ?` eliminator
(`(N-Match)` is unchanged — a `null` / `x` arm still discharges an optional).

---

## Deviations

**OPEN: 1.**  `D-Opt-NoNull` — `(N-Opt)` licenses `τ?` for every τ, and two types have no
representation for absence (a `value struct`, and a TUPLE), so both are refused at the
declaration; the design call is loft#1423.  Every other deviation this doc has carried is
closed; the record is in the companion
[types-history.md](types-history.md).

## Conformance check (how we know a deviation is real)

Each deviation should have a falsifying program — the case where obeying the rule and
obeying the code disagree. Examples on record:

- **D1 (CLOSED):** the four `*_hint` side-channels (`lambda_hint` / `enum_hint` /
  `vector_hint` / `read_target_type`) are consolidated into one `Parser.expected` field with
  shape-dispatching reader methods — one `⇐` channel, not four. (#432's `vector_hint` was the
  symptom of the sprawl.)
- **D4 (CLOSED):** the cbor `read_value` cross-branch `arg` (`arg=bytes[i];
  arg=arg*256+…`) inferred `u8` and overflowed. `(I-Join)` now widens it; the falsifier
  graduated to `tests/scripts/433-ijoin-multiply-assigned.loft`.

When a deviation closes, its falsifying program graduates to `tests/scripts/` and the
entry here is deleted.
