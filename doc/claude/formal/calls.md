<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/calls.md — small-step semantics for function calls (strict)

**Catalogue:** @F3 (scalar/call core), @PLN87 (parameter binding), @PLN89 (differential oracle).

> **Rules then deviations** (see [README](README.md)). This is the small-step relation for a
> **function call and return** — argument evaluation, parameter binding, the frame, and the
> result. It is the most load-bearing form operational.md left unwritten. It extends
> [operational.md](operational.md) (eval order, `return`), [heap.md](heap.md) (the parameter
> binding + return value are heap facts), and [binding.md](binding.md) (a `&` parameter is the
> explicit write-back). Every rule below is a **user-visible contract** verified on both
> backends, not a test artefact.

## The one thing to get right

loft's parameter passing is **not** uniform "by value" or "by reference" — it is by TYPE:

- a **scalar** parameter is a **copy** (mutating it never touches the caller);
- a **heap** parameter (struct / vector / collection) is **shared** — a field/element
  **mutation THROUGH it is visible to the caller** — BUT a **whole-value reassignment** of the
  parameter rebinds **locally** (no write-back).

This is exactly the fact [capabilities.md](capabilities.md)'s raw-write rule rests on ("a write
whose root is a parameter mutates the caller"), and it is the contract a user must know to reason
about a call's effects.

## Notation

Uses [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩` and [heap.md](heap.md)'s heap `H`.
`f(p₁…pₙ) { body }` is a function; a call is `f(a₁…aₙ)`.

---

## Rules

### Arguments evaluate left to right, before the body

```
  (F-Args)   in f(a₁, …, aₙ), reduce a₁ to a value first, then a₂, …, then aₙ — left to right,
             each fully, BEFORE the body runs.  Any side effect in an argument happens in that
             order (operational.md E-Left, lifted to n-ary calls).
```

**In words.** All arguments are evaluated, left to right, before the function body begins — so
`add(tag("A"), tag("B"))` prints `A` then `B`, then calls `add`. A call is strict (call-by-value
in the evaluation-order sense: arguments are values before the body sees them); loft has no lazy
arguments.

### Arity — every required parameter must be supplied (static)

```
  (F-Arity)   a call `f(a₁…aₖ)` is WELL-FORMED (checked before the program runs) only if every
              parameter is filled.  A parameter is filled by a positional argument, a named
              argument (`name: e`), or — when no argument targets it — a DEFAULT: a `= e` default
              (bound to e) or a nullable type `τ?` (bound to null).  A parameter with NEITHER a
              default nor a nullable type is REQUIRED: omitting it is a COMPILE ERROR (too FEW
              arguments), and supplying more arguments than parameters is a COMPILE ERROR (too
              MANY).  A compiler-inserted slot (a return buffer) is not a user parameter and is
              exempt from the requirement.
```

**In words.** Every parameter must get a value. A parameter is optional only if it declares a
default (`= e`) or is nullable (`τ?`, which defaults to `null`); otherwise it is required, and
omitting it is a compile error — `missing argument for parameter '…' — the call supplies too few
arguments` — just as passing extra arguments is (`Too many parameters`). loft does **not** silently
fill a missing argument: an earlier "defaulted-null" lenience (a missing argument quietly became
`null`/empty) was **removed** (2026-07-17), because it was a footgun — a missing function-typed
argument filled a broken value and crashed the stdlib, and a missing scalar silently read `null`.
Named arguments may fill parameters out of order and skip a middle parameter that carries a default
(`f(a: 1, c: 3)` when `b` has one). Verified both backends. (Arity is about the *count*; a supplied
argument's *nullability* is checked separately — passing a `τ?` value into a non-null parameter
warns, or errors for a narrow width: [types.md](types.md)'s `N-Store` rule at the call-argument
site.)

### The call binds parameters and yields the return value

```
  (F-Call)   ⟨f(v₁…vₙ), σ⟩ → ⟨r, σ'⟩
               where a fresh frame binds pᵢ per F-Param* below, body runs to a return value r,
               and the frame is dropped (its owned locals freed, heap.md H-Free).
  (F-Return) `return e` exits the current call with e; a function whose body ends in an
             expression returns that expression (the implicit tail return).
  (F-Drop)   …unless the function is DECLARED with no return type: then a value-typed tail
             expression is a STATEMENT.  It is evaluated for its effects, its value is
             discarded, and the function returns nothing.
  (F-Block)  the same rule one level in: a `{ … }` block ending in an expression YIELDS that
             expression, wherever the block stands, and the value flows out to whatever reads
             it.  It is discarded only where the BLOCK itself is a statement — a `;`-terminated
             one, a loop body, or the body a function drops by F-Drop.
  (F-Rec)    a call to the same (or a mutually-recursive) function gets its OWN fresh frame —
             recursion is ordinary, bounded only by the stack.
```

**In words.** Calling runs the body in a fresh frame with the parameters bound, and the value the
body returns is the call's result; the frame's own temporaries are released when it returns.
`return e` leaves early; a function whose last statement is an expression returns it without an
explicit `return`. Recursion is just a call with a fresh frame.

`(F-Block)` is what `(F-Return)` and `(F-Drop)` are the two function-level cases of, and it
is written down because it did not hold: `fn f() -> integer { { 5 } }` answered `null` on the
interpreter and `0` on `--native`, and `fn g() -> integer { n = 5; { n } }` answered `5` on
one and `0` on the other. A block reaching the expression parser is parsed against `Void` —
a statement, as far as that site can tell — so its tail was dropped even where its value was
the function's. The type flowed out correctly the whole time, which is why the function
type-checked and nothing said anything (loft#1076).

`(F-Drop)` is the edge `(F-Return)` alone cannot answer, and it was written after the two
backends disagreed about it (loft#1075). `fn main() { store_persist_copy(h, "…") }` ends in a
call that returns a `boolean`, and `main` returns nothing — so "returns that expression" has
nowhere to put it. The answer is the one the language already had everywhere else: a value
expression in statement position is evaluated and discarded, and the tail of a void function is
a statement like any other. The tail still RUNS — the discard is of the value, not of the work —
and `f()` and `f();` are the same program. Nothing it owns survives the call (measured: a
heap-returning tail discarded 800 000 times holds a flat resident size).

### Parameter binding — by TYPE, not uniform

```
  (F-ParamScalar)  a scalar parameter (integer/float/single/boolean/character) is bound to a
                   COPY of the argument.  `p = e` inside the body is a local update; the caller's
                   value is UNCHANGED.
  (F-ParamHeap)    a heap parameter (a struct / vector / keyed collection) is bound so that the
                   parameter and the caller's argument share the same store: a MUTATION THROUGH
                   the parameter — `p.field = v`, `p[i] = v`, `p.field += …` — IS VISIBLE to the
                   caller.
  (F-ParamGrow)    STRUCTURAL change is mutation, not replacement, so it is visible on the same
                   terms: `p += [x]`, `p.remove(i)`, `p.clear()` and a re-key all change the
                   shared store and the CALLER SEES THEM, at every container kind (vector,
                   hash, sorted, index) and with no `&`.  `&` is not what makes a callee able
                   to GROW its argument; nothing does, because the argument was never copied.
  (F-ParamRebind)  a WHOLE-VALUE reassignment of a heap parameter — `p = [..]`, `p = other` —
                   rebinds p LOCALLY (a fresh backing); it does NOT write back to the caller
                   (@PLN87 P2.4).  The distinction is mutate-through (visible) vs replace (local),
                   and it is the WHOLE distinction: replace is the one thing `&` adds
                   (F-ParamRef), and it is the only local one.
  (F-ParamRef)     a `&`-typed parameter (binding.md) is the EXPLICIT write-back channel: a
                   whole-value `p = e` on a `&T` parameter DOES write through to the caller.
                   It is TRANSITIVE: passing a `&T` parameter on to another `&T` parameter
                   forwards the reference rather than dereferencing it, so the innermost
                   reassignment reaches the outermost caller at any depth.  A forwarder
                   therefore needs the `&` even though its own body never assigns.
```

**In words.** What a call can do to its arguments depends on the type. A **scalar** argument is
copied — the callee can never change the caller's number. A **struct/vector** argument is
**shared**: if the callee writes a field or element (`e.hp = 0`, `v[i] = x`), the caller sees it —
this is deliberate (no deep copy on every call, and it is how a mod "manages the entities it is
given"). Growing or shrinking it is the same kind of act on the same store, so `v += [x]`,
`v.remove(0)` and `v.clear()` are visible too. But **replacing** the whole value (`v = [1,2]`)
only rebinds the callee's local name; the caller keeps its value. To get write-back on a
whole-value assignment, the parameter must be declared `&T` ([binding.md](binding.md)) — that is
the one explicit channel. (Verified: `e.h=99` in a callee ⇒ caller sees `99`; `n=n+1` on a scalar
⇒ caller unchanged; `v=[9,9]` on a plain vector param ⇒ caller unchanged.)

**A rebind need not be WRITTEN in the body it is local to.** `(F-ParamRebind)` is about the
binding, not about the spelling: a plain heap parameter handed to a `&` parameter is rebound by
the CALLEE's write-back, and the rule reads the same — the caller two frames up keeps its value,
and the frame the write-back landed in sees the fresh one for the rest of its body.

    fn replace(b: &B)      { b = B { items: [9, 9] }; }
    fn forward(b: B)       { replace(b); }              // b rebinds LOCALLY
    fn main() { a = B { items: [1,2,3] }; forward(a); }  // a is still [1,2,3]

That is also where the fresh store's owner is: `ownership.md` `(O-Latest)` puts ownership on the
LATEST assignment to a binding, and the write-back IS one. Both halves were missing — the store
the binding stopped naming was released by the callee, which cannot see that a plain heap
parameter's store belongs to a frame below it, and the fresh one was owned by nobody (loft#1287,
`heap.md` H-Free's `free_protected` side condition).

**A PARAMETER is not a BIND, and reading one as the other is what put the opposite claim into two
shipped documents.** `binding.md` `(B-Copy)` says a plain bind COPIES — `c = b; c += [4]` leaves
`b` at its old length — and a SLICE is a fresh vector for the same reason, so `w = a[1..4];
w += [9]` leaves `a` alone. Neither fact is about parameters. A parameter aliases
(`F-ParamHeap`), which `binding.md` `(B-Ref-Reshape)` already says in as many words: *"A plain
PARAMETER is NOT exempt: it aliases the caller's element exactly as a `&` one does (calls.md
F-ParamHeap), so the rule keys on the aliasing relation, not on the token."* LOFT.md § Ref-param
vector append and the reference's Vector chapter both promised that `v += [x]` on a non-`&`
parameter was callee-local; it never was, on either backend, and the rules said so from the other
end (loft#1251). Pinned by
`tests/scripts/1251-a-heap-parameter-is-shared-not-copied.loft` — a table over
{append, element write, remove, clear, replace} × {plain, `&`} × {vector, hash, sorted}, plus the
bind and slice controls that make the three cases distinguishable.

**What `&` buys on a collection, given that.** Exactly one thing a caller can observe: whole-value
REPLACEMENT (`v = [7,7]` writes back through a `&vector<T>` and does not through a plain one).
That is worth knowing, because "if a plain parameter grows the caller's vector anyway, `&` adds
nothing observable" is the natural next thought and it is false. `&` is still worth writing where
a signature should say *"this function replaces what you hand it"*; it is not the permission slip
for appending.

**`F-ParamHeap` has a static consequence at the CALL SITE.** Because a heap parameter aliases the
caller's argument, handing a callee both a container and a reference INTO that container
(`f(v[i], v)`) gives it two names for the same store — and if `f` removes from the container
parameter, the write through the other one is lost. That call does not compile
([binding.md](binding.md) `B-Ref-Reshape`, @PLN130 F9 / loft#779). Note it is `F-ParamHeap`, not
the `&` spelling, that makes this a hazard: a PLAIN struct parameter aliases the caller's element
exactly as a `&` one does, so both spellings are refused. This is the only static rejection at a
call site besides `F-Arity`.

### The return value is independent

```
  (F-Ret)    the value a function returns is a FRESH, independent value: a caller that mutates
             one call's result does not affect another call's result, nor any of the callee's
             (now-dropped) locals.  A function that returns a whole heap value hands out an
             OWNED value (heap.md H-Alloc / the return-buffer), never a view of a local.
             EXCEPTION: a `&T` return (binding.md) is an explicit borrow of an addressable input.
```

**In words.** Two calls to the same function give you two independent results — mutating one never
shows up in the other (verified: `a = mk(); a[0]=99; b = mk()` leaves `b[0]==1`). A function never
leaks a view of its own local: the returned heap value is owned by the caller. The only borrow you
can get back is an explicit `&T` return, which binding.md governs.

**A generic's instance owes exactly this, and nothing in the rule distinguishes it.**  The
declaration defers a generic's return promotion to instantiation, and for a long time nothing at
instantiation received it, so `fn same<T>(x: T) -> T { x }` bound to a struct, a vector or a keyed
collection handed the argument up while its concrete twin copied — measured on the 48-cell
independence matrix the sentence above states, 13 generic cells wrong on both backends and every
concrete one right (D-call-13, QUALITY.md B7t).  The concrete twin is the oracle for the instance.

### A method belongs to a type, and a `?` is not part of that name

```
  (F-Recv)   a method's RECEIVER names the type the method belongs to, and the receiver's
             NULLABILITY is not part of that name: `fn m(self: τ?)` is a method of τ.  Two
             overloads may differ in it — the def key carries the `?` (@PLN25), so `m(τ)` and
             `m(τ?)` are two definitions — but the SET of methods of τ is the same set as the
             methods of τ?, and every site that enumerates it sees both spellings.
```

**In words.** Writing `fn area(self: Square?)` declares `area` on `Square`; the `?` says the
implementation tolerates an absent receiver, not that it belongs to another type. A call on a
dense receiver reaches it when no dense overload is declared, and a call on an absent one
reaches it with `self == null` — which is the whole point of the spelling.

The site this rule is about is the one that ENUMERATES: the synthesised variant dispatcher
(@F20) walks the implementations of an enum's variants, and asked for a bare
`Type::Reference(v)` it saw none of the `τ?` ones. The dispatch then fell to the
no-variant-matched tail and answered another variant's bytes on `--interpret` and `0` on
`--native`, while the DIRECT call on the same variant was right — and the
"no implementation of `area` for variant `Square`" warning named an implementation written
five lines above it (loft#1427). One home answers it: `Data::receiver_def_nr`.

*Anchors:* `Data::receiver_def_nr` (`src/data.rs`), its three readers in
`src/parser/definitions.rs` (`enum_fn`, `enum_numbers`, `warn_missing_enum_variants`) and
`one_implementation_per_variant`, which keeps a variant's DENSE overload where both are
declared — the one a direct call takes; `tests/scripts/1427-a-nullable-receiver-implements-its-variant.loft`,
`1427b-…`, and the warning half in `tests/parse_errors.rs`
(`nullable_receiver_implements_its_variant`), which fails on the extra report a `.loft` guard
cannot see.

**What the rule does NOT settle**, and loft#1432 carries: when BOTH overloads are declared, a
nullable receiver still takes the dense one, and the reverse declaration order is refused as a
redefinition — so the two spellings are one method for the redefinition check and two for the
key. This rule is written for the enumeration, where the answer is not in doubt.

---

## Deviations

**OPEN: 0.**

The full register — every closed deviation with its dates and issue numbers, and the
measurement that closed it — is the companion [calls-history.md](calls-history.md).

## Conformance

- **Arg order (`F-Args`)** — `add(tag("A"), tag("B"))` prints `AB` before returning.
- **A block yields its tail (`F-Block`)** — `fn f() -> integer { { 5 } }` is `5`, at any
  nesting depth, for every tail kind (a literal, a local, an operator expression, a call, a
  struct or vector literal) and every type; so is a block as an `if` arm, and one after a
  side-effecting statement. A block bound directly by an assignment (`x = { 5 }`) always
  was. The two legitimate discards are the controls: a `for` body's tail (loft#725) and a
  void function's (`F-Drop`). Guard `tests/scripts/nested-block-in-value-position.loft`,
  both backends.
- **Void tail discard (`F-Drop`)** — a function declared void whose body ends in a value runs
  the tail and returns nothing, for every tail type (boolean, integer, text, struct, vector,
  tuple, a narrow `u8`) and every tail shape (a call, a bare literal, an operator expression, a
  struct literal, an `if`, a nested block), in `main` and in an ordinary function alike. The
  `;` form and a block in VALUE position are the controls. Guard
  `tests/scripts/void-fn-value-tail.loft`, both backends.
- **Arity (`F-Arity`)** — `f(1)` for `fn f(a, b: integer)` (no default) → "missing argument for
  parameter 'b' … too few arguments"; `f(1, 2, 3)` for a 2-param `f` → "Too many parameters"; a
  `b = 5` default or a `b: integer?` nullable parameter may be omitted.
- **Scalar copy (`F-ParamScalar`)** — `fn inc(n){ n=n+1 }` leaves the caller's `x==5`.
- **Heap mutate-through (`F-ParamHeap`)** — `fn mut(e: E){ e.h=99 }` makes the caller's `o.h==99`;
  `fn f(v){ v[0]=99 }` makes the caller's `orig[0]==99`.
- **Heap reassign is local (`F-ParamRebind`)** — `fn re(v){ v=[9,9] }` leaves the caller's
  `o[0]==1`; only a `&`-param would write back. The rule names two spellings, so the oracle
  crosses the RIGHT-HAND SIDE with the type: {struct, struct-enum} × {literal, call, another
  local}, both backends, in
  `tests/scripts/1290-a-heap-parameter-rebind-is-local-in-every-spelling.loft`, with a FIELD
  write (`F-ParamHeap`), a `&` parameter (`F-ParamRef`) and a plain LOCAL as the controls.
  The VECTOR kind's `p = other` spelling was in neither oracle and answered wrong until
  2026-09-05 (D-call-14): `tests/scripts/a-vector-parameter-reassigned-from-a-variable-rebinds-locally.loft`
  crosses it with a returned rebind, a rebind through a value branch, a double one, a loop and a
  read ahead of the rebind, with the mutate-through, a self arm and a `&` parameter as the
  controls.  The COLLECTION half is
  `tests/scripts/1294-a-keyed-parameter-rebind-is-local.loft` — all five keyed kinds
  (`hash`, `sorted`, `index`, `spatial`, `trie` are what `is_keyed` names) crossed with the
  same right-hand sides plus the empty literal, a CONDITIONAL rebind and a double one, with
  the vector kind and an append (`F-ParamGrow`) as its controls. Its whole row wrote back to
  the caller: a keyed `=` lowers to `OpReplaceKeyed`, which deep-copies into the store the
  target's slot names, and for a parameter that is the CALLER's (loft#1294).  The NULLABLE
  row is `tests/scripts/1295-a-nullable-parameter-rebind-has-an-owner.loft`, and it moves a
  different channel: the caller's VALUE was right there all along and the store the callee
  minted had no owner, one orphaned record per call (loft#1295).  `τ?` and `τ` share sentinel
  storage, so a nullable parameter is still a parameter.
  ⚠ **That cross is what this line was missing, and `OPEN: 0` read green over it for a year.**
  The one-cell oracle above asked only `p = [<literal>]`, which was the one spelling with a
  lowering: `p = other` — named in the rule's own text — wrote back to the caller on BOTH
  backends, `p = call()` on the interpreter, and three of the six cells DISAGREED between the
  backends against `(O-NoDiverge)` (loft#1290). A register is only as strong as the oracle
  under it; re-measure before trusting a zero.
- **The `&` write-back leaves the CALLER owning its store (`F-ParamRef` × `B-Copy` ×
  `O-Owner`)** — the rule above says a `&` parameter's whole-value `p = e` writes through; what
  it installs is [binding.md](binding.md)'s question, and `(B-Copy)` answers it: a plain heap
  whole-value source is COPIED, so the store the caller ends up owning must be one no live
  binding in the callee still names.  A bare-VARIABLE source installed the source's own store
  instead, and `(O-Owner)`'s single owner broke in whichever direction the other holder pointed
  — a caller-reachable source (`x = o`, which `(F-ParamHeap)` makes an alias of the caller's
  argument) left the caller's two bindings naming one store and orphaned the displaced one,
  while a callee-LOCAL source (`x = m`) was freed at the callee's scope exit and every later
  read in the caller was a use-after-free.  Both answered CORRECTLY at the call, which is why
  the issue that found the leak scored the value ✔ and never saw the alias: it takes a later
  mutation of the source to see one and a store-slot recycle to see the other.  The oracle is
  `tests/scripts/1303-an-amp-write-back-leaves-the-caller-owning-its-store.loft` — the source
  crossed with the type: {caller-reachable parameter, callee local, field} × {`hash`, `sorted`,
  `index`, `spatial`, `trie`, struct, struct-enum}, with a minting CALL (already correct — its
  buffer is a temp nothing else names, the transfer `(O-Move)` describes), the `vector` kind
  (whose in-place refill is already `(B-Copy)` and has no displaced store), a plain forwarder
  (`F-ParamRebind` — the write-back must still stop there) and a repeated call as controls.
  `x = x` has no cell: the language refuses that spelling, and excluding it from the
  materialisation is what keeps the refusal (loft#1303).
- **Return independence (`F-Ret`)** — `a = mk(); a[0]=99; b = mk()` leaves `b[0]==1`.  The
  NULLABLE record return, which has no delivery buffer, is pinned apart in
  `tests/scripts/1337-a-view-of-a-local-returned-through-a-nullable-return-is-copied.loft`:
  a walker's local rebound through its own reference field, a projection inside an `if` arm
  beside a `null` and beside a literal, the walk that ends at null, eight nullable controls
  and the two dense twins — each read after a filler allocation, both backends
  (`D-call-8`, loft#1337).

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on argument
order, a parameter's effect on the caller, or a return's independence is the definitional error
this doc names.
