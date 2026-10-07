<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# formal/interfaces-history.md — the deviation register for [interfaces.md](interfaces.md)

> **The rules are next door.**  [interfaces.md](interfaces.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **0** (D-gen-10 opened 2026-10-05 and closed 2026-10-06, loft#1872; D-gen-7 and D-gen-8 opened and closed 2026-10-05, loft#1868; D-gen-5 — C91's content `==` reaching `Equatable` — and D-gen-6 — a tuple bound to it, loft#1738 — both opened and closed 2026-09-29 by @PLN175 step 8 and its follow-up).  `D-gen-1` and `D-gen-2` were opened and closed on 2026-08-29, `D-gen-3` and
`D-gen-4` on 2026-09-02.

⚠ **This line read `OPEN: 0` because *"a rules doc adds no code deviation"* — a claim about the
doc's GENRE, not a measurement, and the same sentence `formatting.md` carried for its whole life
until the walk that first asked found four defects there.  It has now produced one here too.**
The oracle under it (`86-interfaces.loft`, `48-generics.loft`, and the numbered scripts) is real,
but it is an oracle for the shapes those files happen to write; `D-gen-1` is what it could not
see.

### D-gen-11 — OPENED AND CLOSED (2026-10-07, loft#1927): a member's RETURN was compared only between named types

`(G-Sat)` satisfies a bound by a function of signature `[Self ↦ C](p̄ -> R)`.  The parameters
are compared (loft#1818); the return was compared only when both sides named a struct or enum
(@PLN125 A2a), so `fn size(self: A) -> float` met `fn size(self: Self) -> integer`, and the
generic, typed with `integer`, read the float's bits (`4612811918334230529` for `2.5 + 1` on
the interpreter, rustc E0308 on `--native`); a `-> text` member printed `50`, a
`-> vector<text>` one answered a handle as an integer on both backends, and a member returning
nothing printed nothing.  `Data::return_fits` compares every return as a parameter is, at
every position of the type, nullability peeled; a member whose interface member returns
nothing may still return a value.  Guards: `1927-a-member-returning-another-type-does-not-satisfy-a-bound.loft`,
`1927b-a-member-returning-the-declared-type-satisfies-a-bound.loft`.

### D-gen-10 — OPENED (2026-10-05) AND CLOSED (2026-10-06, loft#1872): a recursive instance's struct result read inline

`(G-Mono)` says an instance answers as its concrete twin does.  The twin `fn pick(n, a: S,
b: S) -> S` returns `S["a", "b"]` through a caller's buffer; the recursive generic instance
returned a bare `S` with no buffer, and its recursive arm's result was a store of its own that
a caller reading a field straight off the call (`pick(1, a, b).x`) never released — one record
per call on `--interpret`.  The vector half was silent: the instance handed the caller's own
vector up on its borrow arm and the call site's lift freed it.  The instance now binds such a
join into one owned local, and a returned whole vector is always a copy (`(F-Ret)`).  Guard:
`1872-a-generic-instance-answers-as-its-twin-when-read-inline.loft`.  Recorded in the rules
doc as `D-gen-7`, a number this register had already given loft#1868's entry; the reuse let the
register read the closed entry's date and count the open one as closed.

### D-gen-8 — OPENED AND CLOSED (2026-10-05, found by loft#1868's matrix): a compound on an open instance's field

`(G-Type)` says an open instance's field behaves as its twin's.  Three compounds on a
`Bag<T>` field inside the template did not: `b.items += [x]` built the literal INTO the field
and then appended that vector to the field again, so every append doubled it (two one-element
appends left six); `b.name += "x"` appended to a work text that nothing read the field into
or wrote back (a panic on `--interpret`, E0425 on `--native`).  The append test that
recognises a literal built through its place (`rhs_built_into_place`) knew only the tuple
place; it now knows the deferred field too, and a block is "built into the place" only when
its accumulator is never re-pointed, which a comprehension does.  The text compound writes
back through the deferred field write (`assign_text`).  Guard:
`a-compound-on-a-generic-structs-field-writes-that-field-once.loft`.

### D-gen-7 — OPENED AND CLOSED (2026-10-05, loft#1868): a tuple of a type variable held in a collection

`(G-Mono)` applies `[T ↦ C]` to every constant derived from a type.  A template's
`vector<(T, U)>` kept one: the record `__tuple<T,U>`, laid out once with zero-width members
and named by every instance.  A parameter of that type read `null(oob)` silently, a built one
ran without output, text members panicked in the allocator, `--native` refused every cell
(E0308), and `v[0] = (x, y)` was refused outright.  A tuple of a type variable is now an OPEN
instance of the anonymous tuple template, read as `(G-Type)` reads `Box<T>`: each monomorph
closes it to the concrete tuple's record (`Data::close_instance`), its element stride names it,
and its unbox, member reads, member and element writes and destructuring are deferred to the
monomorph.  The tuple member writer also gained the plain-enum arm a deferred write needed.
Two things the closing taught: an open tuple is anonymous, so another template's `(T?, integer)`
shares this one's placeholder — only the open tuples a template's own types name are closed
(closing the rest minted `__nullable<S>` on pass 2, which H5 refused); and an instance whose
field holds a tuple mints that tuple's record with the field, or its layout waited and the
record took a later id than the generated `init()` gives it (`--native` schema drift).
Guard: `a-generic-over-a-vector-of-tuples-of-its-type-variables.loft`.

### D-gen-4 — OPENED AND CLOSED (2026-09-02, loft#1275): the stub key spelled a NAME, not a signature

`(G-Iface)` calls an interface a set of method SIGNATURES and `(G-Sat)` satisfies it per
signature; neither rule keys a method by name alone.  The bound-method stub did —
`t_<LEN><holder>#g_<method>` — so the two arities of `-`, both `OpMin` after the operator sugar,
asked for one name.  The second declaration was silently dropped, no shipped bound could offer
binary subtraction, and `a - b` under any bound was refused with *"operator '-' requires a
concrete type"*.

The arity is part of the key now, **inside the length-counted portion** beside the holder mark.
Putting it after the method instead was built and measured and is wrong: nine sites strip `t_`
and take what follows the first underscore, so the monomorphiser looked for a concrete `sizer#1`,
found nothing, and a one-method `Sizable` answered garbage while the stdlib's `sum<T: Addable>`
panicked.  The arity is part of the KEY and never part of the NAME — which is what
`bound_stub_name`'s own doc block already said about the holder mark, one paragraph above where
the arity was first appended.

**Two things this did NOT close, both of them rules rather than deviations.**

* A NAMED method required at two arities by one bound set is still refused, at the declaration.
  An operator's arity is fixed by its SYNTAX, so `call_op` asks for the exact stub; a method call
  resolves its RECEIVER before its arguments are parsed, so `x.sizer()` has no arity to ask with.
  The refusal names both arities and the cure.
* A CONCRETE receiver's method key carries no arity either, so a user type provides one arity of
  `-` and not both.  That is why binary subtraction ships as `Subtractable` rather than as a third
  requirement on `Numeric`: `(G-Sat)` is structural, so adding a requirement to a shipped
  interface takes satisfaction away from every user type that already provides the other two —
  a breaking change under COMPATIBILITY.md, and the reason the mechanism fix and the surface
  choice are two decisions.

⚠ **Closing it exposed a `(G-Sat)` hole that had been unreachable.**  Satisfaction asked
`find_fn`, which takes a name and a receiver and no arity.  While no interface could declare one
name twice, nothing could reach it; the moment `Subtractable` asked for a two-operand `OpMin`, a
type providing only the UNARY one answered the name, satisfied the bound, and the monomorph
called it with one operand too many and dropped the second — `diff(a, b)` computed `-a` on both
backends with no diagnostic.  That is loft#1274's defect at the satisfaction site instead of the
use site, and it was introduced and closed inside this change: satisfaction now compares the
VISIBLE parameter count and re-asks through `possible_with_signature`, the resolver
monomorphisation already uses.  **A gap that only a new feature can reach is not a gap the old
oracle was wrong to miss — but it is one the feature has to bring its own cell for.**

Guard: `tests/scripts/1275-a-bound-offers-both-arities-of-minus.loft`.

### D-gen-3 — OPENED AND CLOSED (2026-09-02): the type variable's binding was FILE-scoped

`(G-Gen)` says a header `fn f<T: I₁ + … + Iₖ>` **introduces** the type variable — introduces, so
the binding belongs to that header and two functions writing `T` name two variables, each judged
against its own bounds by `(G-Check)`.  The implementation gave the spelling FILE scope: one
attribute-less placeholder definition stood for every `T` in the program, deliberately, because
sharing it is what lets the stdlib's many `<T>` templates resolve against one definition.

Sharing the placeholder is sound for type RESOLUTION, which is what that decision was about.  It
is not sound for the **bound-method stubs**, which hang off the placeholder (`t_<LEN>T#g_<method>`)
and carry a SIGNATURE.  Two headers whose bounds declare one method name differently therefore
shared one stub, the second requirement was dropped, and every call in the second header was
checked against the first header's parameter list:

```
interface HasSize1 { fn sizer(self: Self) -> integer }
interface HasSize2 { fn sizer(self: Self, scale: integer) -> integer }
fn one<T: HasSize1>(x: T) -> integer { x.sizer() }
fn two<T: HasSize2>(x: T) -> integer { x.sizer(10) }
    ->  error: Too many parameters for T#g.sizer
```

**Order-dependent, which is what made the message unreadable.** Whichever header came first owned
the stub; swap the two declarations and the error swapped with them, naming a parameter that lives
in the OTHER function.  Renaming one variable to `U` compiled and answered correctly — the
one-line test that named the axis, and the reason the earlier costing (as an ARITY problem, on
loft#1300) was wrong.

Four spellings reached it, and only the first two were reported:

| two headers both writing `T`, bounds declaring one method name | before | after |
|---|---|---|
| different ARITY (loft#1301) | refused | ✓ |
| the two arities of `-`, which both desugar to `OpMin` (loft#1300) | refused | ✓ |
| same arity, different parameter TYPE | refused — *"expected integer, got text"* | ✓ |
| same parameters, different RETURN type | refused | ✓ |
| identical signatures (the sharing that must survive) | ✓ | ✓ |

The third and fourth rows were invisible to the conflict diagnostic loft#1301 shipped, which
compares parameter COUNTS: a signature is not an arity either.

Closed by keying the placeholder on `(spelling, bound set)` and resolving the spelling against the
ENCLOSING header before the flat namespace — `Parser::cur_type_var_name` /
`Parser::type_var_holders`, read in `parse_type_inner`.  Sharing stays the norm: two headers with
the same bounds still reach one placeholder and one set of stubs.  Guards:
`tests/scripts/1300-a-generic-header-binds-its-own-type-variable.loft` (ten cells, both backends)
and `tests/parse_errors.rs::two_headers_writing_the_same_type_variable_are_two_variables`, which
pins the declaration ORDER because that is what used to decide the answer.

**The oracle could not see any of it, and the reason generalises.** `86-interfaces.loft`,
`48-generics.loft` and every numbered script declare their generics one bound at a time; nothing in
the tree wrote two headers whose bounds share a method name, because the workaround (rename the
variable) is what anyone hitting it does.  A corpus written against the implementation records the
implementation's own habits.

### D-gen-4 — OPEN (loft#1275): one bound set cannot require two signatures of one method name

What `D-gen-3` leaves.  `(G-Iface)` declares an interface as **the set of its method
SIGNATURES**, so an interface naming one method twice at different signatures is well-formed
under the rules, and `(G-Sat)` satisfies each entry separately against `[Self ↦ C](p̄ -> R)`.
`integer` has `OpMinInt` beside `OpMinSingleInt`, so it satisfies both entries of:

```
interface SubNeg { op - (self: Self, other: Self) -> Self
                   op - (self: Self) -> Self }
fn both<T: SubNeg>(a: T, b: T) -> T { -(a - b) }
    ->  error: generic type T: operator 'Min' requires a concrete type
```

The same collision without writing one interface twice: `<T: S1 + S2>` where `S1` and `S2` both
declare `sizer`.  Here the two requirements really are on ONE variable, so the per-header split
cannot separate them — a bound method is reached by NAME (`method_key` and `bound_stub_name`
produce the same string by construction, which is what lets `find_fn` dispatch on it), and the
key carries no arity.

Costed on loft#1300: adding arity changes the method-key convention `find_fn` dispatches on, the
un-mangling in `re_resolve_call` moves with it, and the mangled names are user-visible in
`--native` output.  The refusal names the two arities and the cure that works today, and
`tests/parse_errors.rs::one_bound_set_cannot_require_two_signatures_of_one_method` and
`::one_interface_declaring_both_arities_of_minus_is_refused` pin both spellings, so the day the
key gains a signature both fail and say so.

### D-gen-1 — OPENED AND CLOSED (2026-08-29): the type variable was only found under two formers

`(G-Gen)` writes a generic's shape as `fn f<T>(x: …T…) -> …T…`, and the ellipsis is the rule:
`T` may sit anywhere inside a parameter type.  The DECLARATION read it that way — the check that
the first parameter carries the type variable is `arguments[0].typedef.contains_def(tv_nr)`, which
descends `Type::for_each_child` and therefore knows all seven child-bearing formers.  **The two
reads at the CALL did not.**  `Parser::extract_type_var` (*which* type variable) knew `Vector`;
`Parser::resolve_type_var` (*what it binds to*) knew `Vector`.  So a declaration the parser
accepted was one no call could reach:

| first parameter | before | after |
|---|---|---|
| `T`, `vector<T>`, `vector<vector<T>>` | ✓ | ✓ |
| `T?` | `Unknown function f` | ✓ |
| `(T, T)`, `(T, integer)` | `Unknown function f` | ✓ |
| `iterator<T>` | `Unknown function f` | ✓ |
| `vector<T>?` | `Unknown function f` | ✓ |
| `fn(T) -> …` | `Unknown function f` | ✓ (except `D-gen-2`) |

The diagnostic is the tell: *"Unknown function"* about a function declared three lines above the
call, at every instantiating type — `text`, a struct and every scalar alike, so the scalar axis
this register leans on could not see it either.

Two further homes rewrote `[T ↦ C]` over the same tree with FOUR formers each
(`Parser::substitute_type`, `Function::subst_type`), so `fn(T) -> T` in a LATER parameter was
refused with *"expected `fn(T) -> T`, got `fn(integer) -> integer`"* — the substitution the
message itself asks for.  A third copy, `Data::rewrite_type_opt`, had all seven; a fourth,
`Function::rewrite_unknown`, had five.  **One question, five homes, four different lists.**

**The corpus is why no oracle could see it, and the number is the point.** Across
`tests/scripts`, `tests/docs`, `default/` and `doc/`, **166 generic declarations put a bare `T`
or a `vector<T>` in the first parameter and not one put anything else** — exactly the two arms
the descent knew.  Implementation and tests were written against each other.  Every `T?` guard
in the tree (`1020-*`, `1023-*`) writes `fn g<T>(v: vector<T>, a: T? = null)`, putting the
carrier first; move the `T?` to the front and the same file will not compile.

Closed by deriving all four from the keystone: `Type::map_children` (the SET twin of
`for_each_child`) and `Type::zip_children` (the PAIR twin, for a walk that descends two type
trees at once) are exhaustive, so a new `Type` variant fails the build rather than quietly
staying parametric.  `extract_type_var`'s leaf also became precise — a type-var PLACEHOLDER
rather than any `Reference` — so a first parameter that names a concrete struct beside the
variable (`(P, T)`) answers with `T`.  Guard:
`tests/scripts/a-type-variable-is-found-under-every-former.loft`.

### D-gen-2 — OPENED AND CLOSED (2026-08-29, loft#1175): a fn-ref returning `T` at `text`

`fn f<T>(x: T, g: fn(T) -> T)` is correct at every instantiation measured — `integer`,
`boolean`, `character`, `float`, `vector<integer>`, a struct — and faults at `text` on
`--interpret` while `--native` answers correctly.  A call through a fn-typed slot pushes hidden
`&text` work buffers, and how many is read off the return type where the call is LOWERED, inside
the template, where the return is still `T` and the count is zero.  This is `(G-Mono)`'s
recurring class exactly: substitution rewrote the TYPE and left the COUNT behind.

Closed by DEFERRAL, the cure this register already names for its class: the count is re-asked in
`rewrite_generic_type_defaults`, where `T` is real and the fn-ref variable's type in the
monomorph's own table is concrete, and the buffers are pushed with the same builder and in the
same order as the four parse-time sites.  `args.len() == params.len()` is what says the buffers
are still missing — the visible arguments are all a site pushes when the count was zero — so a
call whose return was already concrete text is left alone rather than served twice.

Two things the deferral needs that the parse-time path gets for free, both already written down
elsewhere in the tree:

- The variables come from `caller_text_buf`, not the shared `__work_N` counter.  This mint
  happens after both passes, and drawing from the shared sequence would shift every later
  `__work_N` (loft#662's class — the reason `collections::callback_call_ref` already mints this
  way).
- A buffer minted after the parse is not declared at the top level, so `scopes::check` scopes it
  to the ARGUMENT block it appears in and frees it there, before the callee fills it.  A
  top-level `Set` is hoisted for each new one, the same replay `patch_tret_callers` performs for
  exactly this reason.  Without it the interpreter was correct and `--native` emitted a
  `String` declared inside the argument block with an empty `OpCreateStack` beside it, which
  does not compile — the divergence appearing on the OTHER backend from the original fault.

⚠ **The obvious cure was built and measured and is wrong.** `Data::fnref_text_buffers`' own doc
says its candidate test is deliberately loose because *"being loose can only mint a buffer nothing
uses, which the pop removes"* — so counting a PARAMETRIC return as a text candidate looks free.
It cured `T = text` and made all six other instantiations abort: a non-text return has no
`__retbuf` protocol for the pop to trim against, so the looseness is safe WITHIN the text family
and not across its boundary.  The guard keeps every one of those six as a cell for that reason.


- **Conformance is differential + directly checkable** — satisfaction is a single static judgment,
  so accept/reject must agree across the drivers (D-op-1's driver-agreement facet). `G-Sat`/`G-Check`
  are checkable directly (a missing method rejects on both backends); the runtime behaviour of a
  monomorphized generic is pinned by `tests/scripts/86-interfaces.loft`, `tests/scripts/48-generics.loft`,
  `tests/scripts/1028-generic-null-typed-per-monomorph.loft` and
  `tests/scripts/1032-generic-iterator-return.loft`.

- **What `OPEN: 0` rests on here — "applied throughout" is the load-bearing phrase.** `(G-Mono)`
  says `[T ↦ C]` reaches *attribute, return, and body types, and every method call*. Four
  defects have now been the same omission: an operation whose choice is a function of `τ` was
  DECIDED while `τ` was still the type variable, and substitution then rewrote the type and left
  the choice behind — loft#1016 (`x?`'s default), loft#1020 (`x == null`), loft#1028 (a `null`
  literal's conversion), loft#1032 (the yield channel a `for` over a generator is paired with).
  Each was invisible to the oracle above, because both scripts instantiate
  over records; none of the three misbehaves at `T = <a struct>`, where a reference sentinel is
  the right answer anyway. loft#1028 is the sharpest reading of that gap: it made the two backends
  disagree — the interpreter answered a `text` monomorph the empty text, `--native` refused to
  compile the program — which is the one thing this section says monomorphization cannot do.
  A scalar instantiation is therefore one axis this doc's oracle was missing, and the count
  stays 0 only as long as the tests keep one.

  **The count is now six, and the two newest were found by sweeping the OPERATION rather
  than the type** (2026-08-22).  Both `1028-*` and `1032-*` sweep `T` across the scalars,
  but each sweeps ONE operation — the null, the yield channel — so the axis left fixed
  was *which operation the template decides*.  Moving it turned up two more the same day:

  - **The `??` null CHECK.**  `== null` was deferred by loft#1020; `??` asks the same
    question and was not.  It took the placeholder's own shape (a reference) and baked
    `rec != 0`, and the after-the-fact repair listed integer / text / float / single /
    enum and ended `_ => None`, so `boolean` and `character` fell through it.  `x ?? fb`
    LOOPED FOREVER at `T = boolean` and corrupted a record at `T = character` on
    `--interpret`; `--native` refused to compile either monomorph.  All three spellings
    were affected — `x ?? d`, `x?`, and `x ?? return d` — because all three reach the
    one check.
  - **The element READ.**  `wrap_vector_get_val` picks the value-extraction op from the
    element type and ended `_ => return code`, which reads as *"everything else is
    reference-shaped"* and was not: `character` and a VALUE enum both need unpacking.
    A template's `v[1]` handed back the address as the value — a garbage codepoint for
    `['a','b']`, `null` for `[Col::Blue, Col::Green]` — on BOTH backends, while the
    concrete twin was right.

  Both are now closed, the check by deferral and the read by an EXHAUSTIVE match (adding
  a `Type` variant fails the build there rather than joining the unhandled set).  The
  guard is `tests/scripts/generic-monomorph-null-and-element.loft`, which pairs every
  boolean and character cell with its hand-written twin — `(G-Mono)` as an assertion
  rather than as a claim.

  **A seventh, from asking the same question of the WRITE side** (2026-08-22).  The
  element read was one operation; the element WRITE is another, and its corpus holds a
  different axis fixed — not the type, and not the operation, but the *spelling*.  P241's
  rewriter re-emits a monomorph's vector writes, and every test of it since 2026-05 uses
  `o += [x]`; nothing used `v[i] = x`.  An append emits a three-op sequence the rewriter
  matches, an indexed assignment emits a LONE `OpCopyRecord`, and that one reached the
  monomorph carrying the type variable's record id: at every scalar type the run PANICKED
  in the allocator, and for a struct parameter it silently wrote nowhere and read the old
  element back.  Closed by routing both spellings through the one setter builder, guarded
  by `tests/scripts/generic-vector-element-write.loft`, which sweeps spelling × type ×
  vector origin.

  The three together say the axis to sweep is not fixed: it was the TYPE for #1028, the
  OPERATION for the `??` check and the element read, and the SPELLING for the write.  What
  they share is the question — *what does this corpus never vary?* — and that question is
  the instrument, not any particular answer to it.  The lesson generalises past this doc: **`_ => None` and
  `_ => return` are how a decision that is a function of `τ` goes missing quietly**, and
  a missing arm looks exactly like a deliberate one until something reads the answer.

  loft#1032 is the same reading a second time, and adds a **third** thing the oracle did not
  carry: a RETURN TYPE that is not the bare `T`. `substitute_type` had arms for `vector<T>`,
  `(T, T)` and `T?` and none for `iterator<T>`, in BOTH twins — the parser's and the variable
  table's — so a generic returning a generator kept the type variable in its return and in the
  handle its caller bound, while the loop variable beside it was substituted. `(G-Mono)` names
  the return explicitly, so this was a deviation and not a boundary; the rule did not move. The
  scalar axis is again what made it visible: at `T = text` or a struct the DbRef yield channel
  is the right answer anyway, so every cell of the new script passes before the fix at those
  types. Two of the three other defects the same repro surfaced were NOT monomorphization
  deviations at all — a forward call's back-patch and `--native`'s argument-hoist path each
  broke for a generator with no generic in the program — which is the loft#1029 lesson
  restated: a generic corpus is where such a thing becomes visible, not where it lives.

  The corpus is thin on a **second** axis, and loft#1029 is how that surfaced: it varies the
  instantiating TYPE and never varies how the ARGUMENT is spelled. Every call in both scripts
  binds its argument to a variable first, and a fresh-arm/borrow-arm join reached with anything
  else — a literal, a field, an element, a `??` — leaked a record on both backends until
  2026-08-20. That defect was NOT a monomorphization deviation — it reproduces with no generic in
  the program at all, so it is `ownership.md`'s to own (D-own-6, now closed) — but it was a
  generic corpus that made it visible, and the same omission would hide a monomorph-only variant
  of it here. `(G-Mono)`'s promise is that a specialised copy behaves as the hand-written
  concrete one would; an oracle that fixes the argument spelling cannot see the cases where it
  would not. The generic spelling itself is now a probe under
  `tests/scripts/1029-inline-argument-borrow-source.loft`'s finding and measured clean.
- **Test-hygiene note (resolved 2026-08-09):** `86-interfaces.loft::test_bounded_for_loop_struct`
  — a bounded `<T: Validatable>` for-loop over a struct vector calling a method per element — was
  commented out under a stale "crashes with P136 (use-after-free)" note. That bug is FIXED and the
  guard is live: `loft --tests tests/scripts/86-interfaces.loft` runs 11 functions including it,
  green. Only the trailing "Uncomment when fixed" comment beside it is left over.

## Carried by interfaces.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [interfaces.md](interfaces.md) now states only what is open.

### D-gen-4's closure summary and the operator-arity residue

**OPEN: 0.**  `D-gen-4` closed 2026-09-02 (loft#1275): a bound-method stub is keyed by
`(name, arity)`, so one bound set holds two SIGNATURES of one name and an interface may declare
`-` at both arities.  The record, and the four closed deviations, are in the companion
[interfaces-history.md](interfaces-history.md).

⚠ **Closed for an OPERATOR, and the residue is a rule the language keeps rather than a
deviation.**  An operator's arity is fixed by its SYNTAX, so the call site asks for the exact
stub.  A named method resolves its RECEIVER before its arguments are parsed, so `x.sizer()` has
no arity to ask with, and one bound set requiring `sizer` at two arities is refused at the
declaration — which is `(G-Iface)` satisfied and a *parsing* order, not a rule bent.  Separately
a CONCRETE receiver has no arity in its method key either, so a user type provides one arity of
`-` and not both; that is why the shipped surface puts binary subtraction in `Subtractable`
rather than adding it to `Numeric`, where it would have taken satisfaction away from every user
type that provides `OpMul` and unary `OpMin` today.

### the status line formal/README.md's area table carried until 2026-09-04

**rules written (2026-07-05), 0 own** — `interface I { fn m(self: Self,…) }`, STRUCTURAL satisfaction (no `impl`), bounded `fn f<T: I>(…)`, parser-side monomorphization (one copy per concrete type → both backends identical), static satisfaction check (`'C' does not satisfy interface 'I': missing m`); compile-time only (no dynamic dispatch / inheritance / associated types — decided edges)

