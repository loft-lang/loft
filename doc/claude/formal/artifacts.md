<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/artifacts.md — a compiled unit is reused only for the code it was built from (strict)

> **Rules then deviations** (see [README](README.md)).  Like [performance.md](performance.md),
> these rules bind the DISTRIBUTION rather than a language construct: when a library's compiled
> unit (`<package>/native-auto/`, built by the C backend and loaded by the interpreter) may be
> adopted by a run instead of built again.  A unit adopted for code it was not built from runs
> other code than the program asked for, with no report: a cure switch that does not reach the
> library, a component a run cannot open, a type table read at the wrong width.  So the rule is
> stated on the INPUTS, and every site that composes a unit's key cites it.  The key is composed
> in one place, `native_adopt::adopt_shared_cdylib`, from the parts in `opt/native_key.rs`.

## Rules

```
  (Art-Key)    a compiled unit is adopted only by a run that would build it from the same
               inputs.  Its key covers every input its code is built from:
                 the loft that generates it — its build and its executable;
                 the packages whose code it carries — by name and version, and the target;
                 the components its C binds — absent, or which library, with the stamp
                   beside it (the C opens a component by the path it was built with);
                 every switch in force that can change the code it is generated with —
                   each `LOFT_*` setting of the environment that is not inert (Art-Inert).
               Nothing a unit is generated from lies outside its key.  What the key leaves
               out is what cannot change the unit's code: where files are, how long a run
               may take, what it logs, and the program around the unit (a hosted unit names
               types by key and translates them where they cross into the stores).

  (Art-Inert)  a switch is left out of a unit's key only when no generator reads it, and the
               switches left out are ONE explicit list, by exact name (no prefix: one family
               holds both kinds), each with what it governs instead: `INERT_SWITCHES`.  A
               switch missing from the list costs a rebuild; a switch on it changes no
               generated code.  The units the release publishes are built under inert
               switches only, so a user's key and the release's agree.

  (Art-Name)   the key is part of the unit's file name, and a unit is adopted only after it
               answers that key itself.  Two keys never share a file, so two runs with
               different inputs never open each other's unit, and a file another key put at
               a name is built again, never adopted.

  (Art-Stale)  a unit older than an edit to a source it carries is not adopted: the run builds
               it again, or interprets while the source is still changing.
```

## Deviations

OPEN: **1**

- **D-art-1 (OPEN, [loft#2654](https://github.com/loft-lang/loft/issues/2654)) — read in code, not reproduced** — violates (Art-Inert)'s last clause in
  effect, not in letter: a unit the release builds for a package that binds a component
  (`server`, `web`, `ssh` bind `net`) carries the BUILD machine's path of that component, and
  its C opens the component by that path alone on a desktop (`loft_ffi_sym` in
  `src/c/loft_ffi_host.h`; only an Android app finds it by file name).  On a user's machine the
  component lies elsewhere, so the published unit could not open it; by (Art-Key) its key
  names the path, so it is not adopted and is built again, which needs a C compiler.  The
  release publishes these units for machines without one.  Closes when a unit opens a component
  where the running loft finds it (`components::path`), and the key then names the component's
  stamp alone.  The owner decides the route.

## Conformance

| rule | guard |
|---|---|
| (Art-Key) — components | `tests/ffi_c_library.rs::a_unit_is_keyed_on_the_components_it_binds` |
| (Art-Key), (Art-Inert) — switches | `tests/ffi_c_library.rs::a_unit_is_keyed_on_the_switches_that_change_its_code` |
| (Art-Inert) — the release | `tests/ffi_c_library.rs::the_release_builds_its_units_under_inert_switches_only` |
| (Art-Name) | the content-addressed name and the probe that asks a unit its key, in `adopt_shared_cdylib` |
| (Art-Stale) | `source_newer_than` in `adopt_shared_cdylib`, and the edit-loop branch beside it |

The history of the key — what each part was added for — is in
[artifacts-history.md](artifacts-history.md).
