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
| P5b | The unit is generated from the library alone and translates the type crossings by key; the verify also compares root types. | A consumer that loads another library first, adopted under the verify; two consumers' units byte for byte. | A store's type or bytes differ, or two consumers build different units. |
| P5c | `loft build-native` with `LOFT_LIB_BACKEND=c` builds the unit; `library-ci-reusable.yml` builds and tests it beside "Native C" and publishes it under `prebuilt/<triple>/`; the interpreter adopts a matching prebuilt with no `cc`. | The library's suite with the prebuilt adopted, `cc` absent. | The prebuilt is not adopted, or an answer differs. |

## Measured

(Filled in as each phase lands.)
