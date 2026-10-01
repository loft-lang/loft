<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/dispatch.md — which definition of a name a call reaches (strict)

**Catalogue:** @F122 (multiple dispatch), @I78 (`Disp-World`), @PLN162 (the plan that built it),
@PLN165 (`D-Rank`, generic members).

> **Rules then deviations** (see [README](README.md)).  A NAME may have several definitions — an
> OVERLOAD SET — told apart by their parameter TYPES.  This chapter says which one a call
> reaches, when that is decided, and what is refused.  It extends [calls.md](calls.md) (a call's
> evaluation, `F-Recv`'s two spellings and nullability routing, `F-OneBody`) and
> [matching.md](matching.md) (`Disp-Match-Equiv` holds a set to its canonical `match`).  The
> register — closed deviations, how each rule landed — is
> [dispatch-history.md](dispatch-history.md).

## The principle

**Where the language could either refuse or choose, it refuses.**  A refusal defines a subset of
valid programs, and widening it later only adds programs; a rule that silently chooses fixes a
meaning that a later revision would change without a word.  `Disp-Ambiguous`, the incomparable
pairs of `Disp-Specific` and `D-Rank`, `Disp-Exhaustive` and `Disp-Hint` are this decision.

## The rules

```
  (Disp-Key)        a definition is keyed by its parameter TYPES, never a parameter's name.  A name
                    with ONE definition keys as before (`n_<name>`, or `t_<τ>_<name>` for a method);
                    a name with several is an overload set: one dispatcher whose members are keyed
                    by their full parameter spelling.  The key is as coarse as a type's key —
                    `vector<integer>` and `vector<text>` spell alike, so two definitions differing
                    only there are a redefinition.  One SOURCE is one set; a library's set reaches
                    every import spelling of its consumers whole, and a consumer's definition of
                    ANOTHER spelling joins it, while one the set already carries is a
                    redefinition.

  (Disp-Applicable) a definition takes a call when every argument may satisfy its parameter, as
                    the call's own argument check (`can_convert`) decides — after (F-Recv)'s
                    nullability routing, so a nullable argument reaches the `τ?` member where
                    one is declared.  A trailing parameter with a default may be omitted.

  (Disp-Specific)   per position, an argument RANKS against a parameter:
                      EXACT      the same type;
                      WIDENED    a variant into its enum (C-Var), an integer into a range that
                                 holds it (C-Int), a present value into `τ?` (N-Intro);
                      LOSSY      `τ?` into `τ` (N-Store);
                      CONVERTED  any other conversion the argument check admits.
                    A nullability step adds to the base rank.  Definition M₁ is more specific
                    than M₂ when it is no worse at every position and better at one.

  (D-Rank)          a parameter naming a type variable ranks GENERIC where it binds: worse than
                    EXACT, WIDENED and LOSSY, and INCOMPARABLE with CONVERTED — `f(x: float)` and
                    `f<T>(x: T)` called with an integer are ambiguous, one needing a conversion and
                    the other an instantiation.  Between two generic members, the one admitting
                    strictly fewer types is more specific — (G-Select), interfaces.md.

  (Disp-Select)     a call reaches the unique most-specific applicable definition.

  (Disp-Ambiguous)  two applicable definitions with neither more specific, and no third more
                    specific than both, are refused at compile time, naming both.

  (Disp-Exhaustive) a call no definition takes, for the STATIC types of its arguments, is refused
                    at compile time, naming the argument types and the definitions declared.
                    Since (Disp-Applicable) is monotone under the variant-into-enum subtyping, a
                    covered call is covered for every runtime value: no "no applicable method"
                    path exists at run time.

  (Disp-Hint)       a name with several definitions offers no parse hint: its arguments are typed
                    from their own spelling, and one that cannot be (an untyped `|x|` lambda, a
                    literal needing a width) is refused at the call, naming the definitions and the
                    typed spelling that cures it.  A name with one definition keeps its hint.

  (Disp-Closed)     selection for a call whose argument types are statically concrete runs at
                    compile time and lowers to a direct call to the selected definition — no
                    runtime table.  A member nothing calls is unreachable and absent from a
                    `--native-release` artefact.

  (Disp-Dynamic)    a call where an argument is held at an enum (dense, or nullable when the call
                    has a static selection) selects at run time on the runtime variants, through
                    one synthesised function per (name, spelling) whose body is the canonical
                    `match`; each leaf is what (Disp-Select) picks for that variant tuple.  A null
                    reaches the static selection.  The enum-level definition is the `_` arm, so a
                    `self` set over an enum with an enum-level member covers every variant; a
                    set without one that misses a variant is refused at the call, in
                    `M-Exhaust`'s shape.

  (Disp-Return)     the definitions of a name may return different types; a call decided at RUN
                    time (Disp-Dynamic) needs the definitions it chooses between to agree, and is
                    refused naming two that do not.

  (Disp-Match-Equiv) every overload set and its canonical `match` — one arm per definition in
                    specificity order, the enum-level one as `_` — select the same body for every
                    argument tuple, on every backend.

  (Disp-World)      under the open profile (`LOFT_LIVE_RELOAD=1`) every call into a set lowers to a
                    per-(name, spelling) function rebuilt when the set grows, so no selection made
                    in an earlier world runs again.  An add that would leave a served tuple
                    ambiguous or uncovered, a removed or re-signatured member, a second definition
                    of a name that was one function, and an add to a set holding a generic member
                    are refused whole.
```

**In words.**  A name may be defined several times for different parameter types, and a call
reaches the one whose parameters fit its arguments most closely: the exact type before a
widening, a widening before a lossy step, a concrete definition before a generic one.  When two
fit equally well, or none fits, the program is refused — the compiler never picks by
declaration order.  The choice is made at compile time unless an argument is held at an enum,
and then it is made on the variant at run time, exactly as the equivalent `match` would.

**Declined: an untyped fallback.**  The design's `Disp-Fallback` — a definition with untyped
parameters, applicable to every call — was not added.  The fallback is a TYPED definition at the
enum (`Disp-Dynamic`'s `_` arm): forgetting it is refused by `Disp-Exhaustive`, where an untyped
total default would make every call covered and the same mistake a silent no-op.  Matching on
VALUES in a definition was declined too (owner ruling): `match` is the home of every
value-shaped decision, and value clauses would make coverage undecidable.

## Where each rule is kept

| rule | site | guard |
|---|---|---|
| Disp-Key | `Parser` key building (`f_` / `t_…#…` keys) | `tests/scripts/a-library-exports-an-overload-set.loft` |
| Disp-Applicable, Disp-Specific, D-Rank, Disp-Select, Disp-Ambiguous | `parser/dispatch.rs` (`dispatch_rank`, `rank_no_worse`, `select_overload`) | `tests/scripts/1811-a-method-overload-set-is-one-set-in-either-declaration-order.loft`, the refusals in `tests/parse_errors.rs` |
| Disp-Exhaustive | `Parser::refuse_uncovered_variants`, the call refusal | `tests/parse_errors.rs` |
| Disp-Closed | selection at parse time | `tests/introspect_dispatch.rs` |
| Disp-Dynamic, Disp-Return | `parser::dispatch::dynamic_dispatcher` | `tests/scripts/a-method-at-the-enum-is-the-wildcard-for-variants-without-their-own.loft`, `tests/scripts/a-nullable-enum-argument-is-dispatched-on-its-variant.loft` |
| Disp-Match-Equiv | the oracle twin | `tests/oracle/34-dispatch-set.loft` / `34-dispatch-set-as-match.loft`, `tests/oracle/35-dispatch-self-set.loft` (`@ORACLE_TWIN`) |
| Disp-World | `live_reload::add_fn_block` | `tests/live_world.rs` |

## Deviations

**OPEN: 0.**  D-disp-1 (the `self` set over variants) and D-disp-2 (a nullable enum position)
are closed; the record is in [dispatch-history.md](dispatch-history.md).
