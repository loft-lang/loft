<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# tests/reference — the code samples of the hand-written reference, as programs that run

Every `loft` fence in [LOFT.md](../../doc/claude/LOFT.md) and
[STDLIB.md](../../doc/claude/STDLIB.md) is a **verbatim window of a file here**, and the
file asserts what the page says about it.  The line above a fence names the file:

```markdown
<!-- from tests/reference/closures.loft -->
```loft
greeting = "Hello";
…
```

`scripts/rule_tags.py fences` reads every fence, finds the window in the named file
(indentation normalised, blank lines kept) and reports one that has drifted or names no
file; `make ci` prints the report, and it gates once the walk is complete (@PLN176).  The
programs run in `make ci` through `wrap::reference` on the interpreter and
`native::native_reference` on `--native`, so a sample that stops being true turns a test
red instead of leaving a stale fence on the page an agent reads first.

## Why

Measured 2026-09-28: LOFT.md carried 65 fences and STDLIB.md 29 with no program behind
any of them.  The first walk found the pages teaching a refused spelling (`map(v, fn double)`),
a variant pattern (`JObject _`) the parser does not take, statements without the `;` a
function body needs, a closure-capture paragraph that contradicted the formal rules and
the closures chapter above it, a stub said to answer null that answers `""`, and a
"say it directly" spelling for moving a keyed record that deletes the record.

## The contract

1. **One file per page section**, named for the section.  A `// @PAGE:` header names the
   page and section it keeps.
2. **A cell is a zero-parameter `fn test_…`**, and it ASSERTS — a printing cell passes while
   answering anything.  The fence's lines sit inside the cell verbatim; the context a
   fence assumes (a struct, a helper, the values it reads) sits above them in the same
   cell or at the top level.
3. **A refusal is its own file**, `<section>-refused.loft`, with `@EXPECT_ERROR`, because
   one file cannot both stop at an error and run its other cells; the page shows the
   refused spelling and the accepted one as two fences.
4. **A script-shaped sample** (top-level statements, no `fn main`) is a file headed
   `// @SCRIPT`; it runs top to bottom on both backends and its top-level `assert`s gate it.
5. **No `use` of a library.**  A sample that needs one is kept by that library's own
   testbed and is marked `<!-- from library:<name> -->` on the page; the checker lists it
   and does not gate it.
6. **A fence that is not a program** — a grammar shape, a signature listing — is a
   ```` ```grammar ```` fence, and the checker does not ask.
7. **Both backends.**  `make test-native` and `native::native_reference` compile every
   file here; a cell that is red on one backend only is a bug to fix, never a cell to drop.
