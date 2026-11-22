<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN206 P5 — libraries build their C artifact; the interpreter adopts it

P4 built a hosted C unit for a library inside the program that uses it, numbered by that
program, and keyed by that program's type layout and function numbering.  P5 makes the unit a
library's own artifact: built once, by the library's CI, adopted by any program.

## The invariant

**A hosted unit means the same in every program that adopts it.**  The unit is generated from
the library alone (the program `use <lib>;`, as `loft build-native` already parses it), so its
code names types by that canonical program's numbers.  Those numbers are the unit's own: they
never reach the adopting program.  Every number that crosses into the interpreter's stores or
out of them is translated by key (`lib_ids::TypeNames::key`, the keys the rustc artifact
already resolves through `HostTable::type_id`), at load, into a table the unit reads.

## Where a number crosses

A type number lives in three places, and only one is shared:

| Place | Whose | Crosses? |
|---|---|---|
| a literal in the unit's code (`loft_database_typed(…, 100)`), its type table | the unit's | no |
| a store's root type (`Store::known_type`, the C `known`) | shared | yes: written and read by both |
| a root record's tag (byte 4, `OpDatabase`, D47) | shared | yes: the store's bytes |

A record's other bytes hold no type number: a stored reference is store, record and position,
an enum value is its discriminant.  So the C runtime's writes and reads of `known` and of the
root tag are the whole boundary: in a hosted unit they go through two translations,
`loft_tp_out` (unit to host) and `loft_tp_in` (host to unit), filled at `loft_shared_init`; outside
one they are the identity, and the code a `--native` program compiles is unchanged.

A function number is the same question.  A unit whose code names one of its own functions by
number (a function value made inside it) would need the same translation through
`HostTable::fn_id`; P5 starts by declining such a function at the bridge (it interprets), and
measures how many there are before building the translation.

A source path in a fault site is the third program-specific text: it names the package's file
by the library's name (`<lib>/src/<file>.loft`), as the interpreter's report names a library
file.

## What else the unit must share with the interpreter

Two more things a unit built for any program has to hold, both found by the sweep (P5a):

- **It is an OPEN unit.**  Its exports are called by an interpreter the writer never sees, so
  it takes no whole-program rewrite that changes a signature (`Cx::open`, as an `open_world`
  program takes none): a value record returned as a struct by value is not what the bridge
  calls.
- **A `[c] shim` is one image.**  The interpreter opened the shim's shared library for the
  parse; the unit links that library by its name, never a copy from the shim's archive, so a
  shim's state (a generator, a device table, a handle counter) is one in the process.
- **Its constants are its own.**  A library constant is built where the unit is generated (the
  library alone), not taken from a consumer's compile, which builds only what it reaches.

**What the open unit costs.**  A rewrite admitted by facts about every caller is not admitted in
a unit whose callers are unknown.  `@FR-H-TextReplace` is the one the sweep met: a text field
written anew keeps the text it replaced until its store dies, where the interpreter, which
knows the callers, writes in place.  The value is the same and placed otherwise; a library
that rewrites a text field in a loop grows its store by the old texts.  Giving the unit the
admission it can prove from the library alone (a parameter no export hands out) is the cure,
measured before it is built.

## What the verify cannot compare

A call run twice — compiled on a copy of the world, interpreted on the world — answers alike
only when everything it touches is in the stores.  A call that reaches a C function (a `#c`
binding, a library native) touches that function's own state, which no copy holds: it is made
once, and counted under "made without a comparison", as a call with a tuple argument and one
with a file-backed store live are.  Such a library is checked by its suite's own answers
(P5a's interpreted-against-adopted output), not by the per-call comparison.

A store whose bytes differ is compared again by value: its root rendered through its type
(`Stores::show`), which follows references rather than record numbers.  The same value placed
otherwise is counted ("alike in value, placed otherwise"), not failed; a store with no known
type fails on its bytes.  The verify's own control, `LOFT_HOSTED_VERIFY_CONTROL`, interprets the
call on both sides: a difference it shows is the copy's, not the unit's.

## Failure paths

1. **A crossing that is not translated.**  The unit writes its own number into a store's
   `known` or root tag, and the host reads another type — silently, until a `sizeof`, a `show`
   or a parse reads it.  Caught by: the verify (`LOFT_HOSTED_VERIFY`) comparing every store's
   root type as well as its bytes, run in a consumer whose numbering differs from the unit's
   (another library loaded first).
2. **A key the program lacks.**  The program does not declare a type the unit names: the unit
   is refused at `loft_shared_init` and the library interprets, as a rustc artifact is refused.
3. **An artifact for another runtime.**  A unit built by another loft names a store layout or a
   host ABI this one does not have: refused by the layout symbol and `HOST_ABI`, as today.
4. **A literal that is the program's after all.**  A writer site prints a number taken from the
   consumer's tables while building in the consumer.  Removed by construction: the unit is
   generated only in the canonical program, so there is no consumer table to read.  Guard: two
   consumers numbering differently build byte-identical units.

## The artifact's key

Not the loft executable's identity and not the program's layout, as a rustc artifact's is: the
C runtime the unit carries (its headers' hash), the store layout and `HOST_ABI` it was built
against, the library's sources (the contributing set), and the build switches
(`LOFT_POISON_CLAIM`, the plant).  A loft whose C runtime and store layout are the same adopts
it; any other rebuilds it, or interprets where no `cc` is found.

## Phases

| | Work | Compared against | Goes red when |
|---|---|---|---|
| P5a | `scripts/hosted_suite.py`: every library's tests as consumer programs, interpreted and adopted under the verify; the verify reports its reach (calls compared, the unit's coverage). | Each test file's output and exit, interpreted. | A file answers differently adopted, or the verify stops at a call. |
| P5b | The unit is generated from the library alone (`native_artifact::parse_library_alone`, shared with `loft build-native`) and translates the type crossings by key; its key carries no program; the verify also compares root types, the answer's included. | A consumer that loads another library first adopts the unit another consumer built, under the verify. | A store's type, its bytes or an answer differs, or the second consumer builds its own unit. |
| P5c | `loft build-native` with `LOFT_LIB_BACKEND=c` builds the unit; `library-ci-reusable.yml` builds and tests it beside "Native C" and publishes it under `prebuilt/<triple>/`; the interpreter adopts a matching prebuilt with no `cc`. | The library's suite with the prebuilt adopted, `cc` absent. | The prebuilt is not adopted, or an answer differs. |

## Measured

- **P5b** — `tests/hosted_unit.rs`: a unit built by one program is adopted by one that loads
  another library first (`Bag` is type 100 in the first, 107 in the second) and verifies; with
  `loft_tp_out` struck it stops at the first record answer.  The C runtime reaches `known` only
  through `LOFT_KNOWN` / `LOFT_SET_KNOWN` (a scan, falsified).
- **P5a** — `scripts/hosted_suite.py` over the five library repositories' `origin/main` (199
  test files): 123 the same call by call, 44 the same by their output (every call reached state
  outside the stores), 27 never reaching a unit, 5 that run only inside their package (a module
  named as the package's own, a dependency of another name), **none differing**.  On the way
  there the sweep found, in the unit: a value-record signature the bridge did not call, a shim
  linked as a second copy, an iterator crossing the bridge, constants taken from the consumer;
  and in the verify: a world it went on in after freeing the caller's views, a twin answering
  where no caller reads, a copy that restarted the lazy sweep, a nested verify, calls touching a
  file or a generator run twice, and placement taken for a wrong value.
- **P5c** — every package of the five repositories built by `loft build-native` on the C
  backend (46 of 50; `crypto` declined whole, passing as interpreted; `server`, `ssh`, `web`
  have a Rust native crate, owed a `[c] shim` in P6), then every test file run adopted with
  **no C compiler on the path** (`hosted_suite.py --prebuilt`): 114 the same call by call, 43 by
  output, 37 not reaching a unit, 5 package-only, none differing.  The run found that a `[c]
  shim` could not be found without a compiler (its name carries the compiler's identity): it is
  now found by its sources key.  A unit built in two directories is byte-identical
  (`Cx::source_name`), so the unit a library's CI builds is the one a user's machine would.
  Library CI's **Hosted C** step (`library-ci-reusable.yml`) is the gate; tried on `drawing` in
  that shape (its own repository on `--lib`, its other dependencies from the registry): 6 of 6
  the same.  Shipping the units with a release (the release's artifacts, the registry, `loft
  install`) is P6.
