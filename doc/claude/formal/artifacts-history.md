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
