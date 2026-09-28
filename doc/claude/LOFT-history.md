<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Language reference — the record

Limitations [LOFT.md](LOFT.md) and the loft-write skill once stated and the language no
longer has, oldest first.  The pages describe loft as it is now, because a reader learning
the language is not helped by knowing what it could not do last month; this file keeps the
record so a reader who meets an old program, an old workaround, or an old claim can place
it.  Each entry names the issue that closed and the guard that asserts the current
behaviour.  A record doc: its dates are the point (DOC_CONTRACT rule 31).

A limitation still stated on a page cites an open issue, or the rule that makes it a
decision; `rule_tags.py claims --issues` (the `stale-claims` nightly, @PLN176) names any
whose issue has closed, and the entry it turns into belongs here.

## 2026-06-10 — the same `pub fn` name in two modules collided in native code (loft#305)

Two modules each declaring a `pub fn` of one name produced one Rust symbol, and the cdylib
failed with `E0428`.  The symbol now folds a module hash.  Guard: `tests/imports.rs`.

## 2026-06-11 — swapping two `vector<STRUCT>` elements through a temp stays a rule (loft#338)

`tmp = v[j]; v[j] = v[k]; v[k] = tmp` duplicates one element, because `tmp` is a VIEW of
slot `j` and the first assignment overwrites what it views.  Closed by design: a bind is a
view (`(B-View)`, `formal/binding.md`), so the skill keeps the rule without the issue —
this entry records why it is a rule and not a bug.

## 2026-07-29 — a write through a private copy of a `vector<Struct>` field was silent (loft#670)

`w = s.rows; w[0] = …` lands in the copy and is lost, and nothing said so.  The dead-store
lint now warns on the simple shape.  Guard: `tests/data/dead_store_670.loft`, driven by
`tests/dead_code_lint.rs`.

## 2026-08-07 — a text-keyed `spatial` was accepted and then answered null (loft#799)

`spatial<Word[w]>` with a `text` key compiled and every point lookup answered `null`.  It
is refused at the declaration, naming `trie<Word[w]>`.  Guard: `tests/parse_errors.rs`.

## 2026-08-07 — a spatial box query answered points outside the box (loft#800)

`xs[(x1,y1)..(x2,y2)]` answered the raw Z-order interval between the corners, a strict
superset of the box, because the curve threads out of the box and back.  The box form
answers exactly what is inside it.  Guard: `tests/scripts/800-spatial-box-containment.loft`.

## 2026-08-16 — an `i32` field silently truncated a 64-bit integer (loft#931)

`i32` was the one narrow alias the range check could not see, so a plain `integer` written
into an `i32` slot lost its high bits.  The narrowing check is range containment OR a drop
in storage width, which covers every narrow alias (@PLAN48 / @P370).  Guard:
`tests/scripts/931-i32-narrowing-is-checked.loft`.

## 2026-08-19 — a backtick block with an interpolation was not dedented (loft#990)

A `{…}` inside a backtick block switched dedenting off for the whole block, which broke
the shape the feature exists for, templates.  Interpolation now dedents like anything
else.  Guard: `tests/scripts/990-backtick-dedent-with-holes.loft`.

## 2026-08-19 — `x#break` on a non-loop local was an internal compiler error (loft#998)

`Variables::loop_nr` returned the chain length when it found nothing, and `Scopes::scan`
underflowed a `usize` on it.  It is a diagnostic naming the loop variables that can be
written.  Guard: `tests/parse_errors.rs`.

## 2026-08-20 — a `self` method as a fn-ref was "Unknown variable" (loft#1008)

Handing `m_only` (a `self` method) to `map` or a `fn`-typed parameter said the name did not
exist; the `both` spelling said the argument was null.  It is one refusal that names the
cure, wrap it in a lambda.  Guard: `tests/error_messages/cases/56_method_is_not_a_fn_ref.loft`.

## 2026-08-20 — the open spatial walk was the Morton tail, not an outward walk (loft#1002)

`xs[(x,y)..]` and `xs[(x,y)..:n]` yielded only records whose Morton code was at or past
the query's, so a record just behind the query was never returned however close it was,
and a query at the far end of the map answered nothing.  Both forms walk outward with two
cursors seeded either side of the query.  Guard: `tests/scripts/48b-spatial-slice.loft`.

## 2026-08-21 — a withheld `catalogue::f()` read as "Unknown library" (loft#1043)

After `use self::catalogue;` the short name gives no qualifier on purpose (the flat
`catalogue::` slot belongs to the dependency graph), but the refusal said *Unknown
library*, which read as *the module is gone* and cost a tree-wide rewrite to diagnose.  The
compiler now says at the call site that the qualifier is withheld and why.  Guard:
`tests/module_name_clash.rs`.

## 2026-08-21 — `parallel { }` compiled to nothing on `--native` (loft#1054)

The block emitted no code, so the arms never ran and the program exited 0 having done
none of the work.  Guard: `tests/scripts/1054-parallel-block-arms-run.loft`.

## 2026-08-22 — a short lambda could not be a struct-literal field value (loft#1067)

Short-form parameter types were inferred from an expected `fn` type only in some
positions; a struct field's declared type was not one of them.  Any position that names
the signature infers.  Guard: `tests/scripts/1067-lambda-expected-type.loft`.

## 2026-08-24 — one file reachable under two names was parsed twice (loft#1080)

A program outside a package loaded a module flat, a file inside it computed the qualified
key, and the loader parsed the same file again: bare calls became ambiguous against a
module nobody wrote (`src2::part_list`), and a native build emitted every duplicated
function twice under one identifier, 55 × `E0428` with identical hashes — a same-file
collision, unlike loft#305's two files.  The loader asks whether the FILE is loaded, by
canonical path.  Guard: `tests/imports.rs`.

## 2026-08-30 — `fn main` with any other parameter shape was accepted and never filled (loft#1172)

A `main` taking anything but one `vector<text>` compiled and ran with its parameters
silently empty or garbage.  Every other spelling is a named refusal.  Guard:
`tests/main_signature.rs`.

## 2026-09-01 — the reference said a plain vector parameter's append was local (loft#1251)

LOFT.md's ownership paragraph claimed an append through a plain `vector<T>` parameter
stayed in the callee, which the rules never said: a heap parameter is SHARED, and `&` buys
only whole-value replacement.  The paragraph was corrected from the rule.  Guard:
`tests/scripts/1251-a-heap-parameter-is-shared-not-copied.loft`.

## 2026-09-02 — a closure could not capture a `&` parameter (loft#1276)

Every capture of a `&` parameter, read or write, raised four errors about constructs the
program did not contain, one of them advising to remove the `&`.  A closure captures the
POINTEE — a `&S` / `&vector` shared, a `&` scalar copied — and the one refusal left is a
write to a captured `&` scalar, named with its cure.  Guard:
`tests/scripts/1276-a-closure-captures-a-ref-parameter.loft`.

## 2026-09-08 — a variant or literal after a slice `..` was a parse-error cascade (loft#1419)

Only a bare name was accepted after the rest; `[Kw { word }, .., End { e }]` bound the
name `End` and choked on `{`.  The tail admits the same element forms as the head.  Guard:
`tests/scripts/1419-a-fixed-pattern-after-a-rest-is-a-tail-element.loft`.

## 2026-09-08 — a `&` alias or parameter to a keyed collection dropped or refused the append (loft#1433, loft#1445)

`a = &h; a += [rec]` on a `hash` / `sorted` / `index` / `trie` / `spatial` took the
deep-copy path, so the append landed in the alias's own store and `len(h)` read 0; a
`&hash<…>` parameter was refused with a message about a vector.  Both spellings are live
links.  LOFT.md carried this as a "known exception" with a workaround for three weeks
after the fix, which is the case that started @PLN176.  Guards:
`tests/scripts/1433-a-keyed-alias-is-a-link-not-a-copy.loft`,
`tests/scripts/1445-a-keyed-parameter-appends-through-its-link.loft`.
