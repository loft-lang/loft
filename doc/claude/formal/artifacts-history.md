<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/artifacts-history.md — the record for [artifacts.md](artifacts.md)

> **The rules are next door.**  [artifacts.md](artifacts.md) states what must always be true of
> when a compiled unit is reused; this file is its TIMELINE — what each part of the key was
> added for, and what was measured.

## How the key grew

- **Its name (loft#715, loft#717).**  Two runs whose type tables differed adopted each other's
  unit, and a garbage type index aborted the process; the key went into the file name, and
  the unit is asked the key it was built under before it is adopted.
- **The packages (loft#1776, @PLN199 G1b).**  Two runs that resolved a `use` to different
  versions built different code under one layout; the contributing packages' names and
  versions joined the key, and the loft executable's identity with them.
- **The components (loft#2652, 2026-11-26).**  A unit built while a component was absent baked
  in a refusal, and every later run adopted it and kept refusing after the install.
- **The switches (2026-11-26).**  Measured on `main` fb49c3f12: a package built under no
  switch, under `LOFT_NO_PUSH_FILL=1` and under `LOFT_WIDTH=32` left ONE unit, while the C of
  the three builds differed (the cure switch changed the push loop; the width changed the
  integer widths and the type table).  The key had held four hand-picked switches; it now holds
  every `LOFT_*` setting that is not inert (`INERT_SWITCHES`), and the rule was written then,
  as this chapter.
- **The component's place (loft#2654, 2026-11-26).**  The key named the library's path as well
  as its stamp, because a unit's C opened a component by the path the generating machine found
  it at.  A move kept the unit right but cost a rebuild — and a unit the release built for a
  package binding a component (`server`, `web`, `ssh` bind `net`) never matched a user's
  machine, so it was built again there, which needs a C compiler the release publishes the
  units to spare.  A hosted unit now asks the running loft where the component is
  (`HostedTable::component`), and the key names the stamp alone.

## Deviations carried by artifacts.md until 2026-11-26

- **D-art-1 (CLOSED, [loft#2654](https://github.com/loft-lang/loft/issues/2654))** — violated
  (Art-Inert)'s last clause in effect: a published unit for a package binding a component
  carried the build machine's path of it, and its C opened the component by that path alone
  on a desktop (`loft_ffi_sym` in `src/c/loft_ffi_host.h`; only an Android app found it by
  file name), so its key named the path and a user's run built it again.  Closed when the
  hosted unit asks `HostedTable::component` and the key dropped the path; guarded by the
  moved cell of `a_unit_is_keyed_on_the_components_it_binds` (one unit after the move).
