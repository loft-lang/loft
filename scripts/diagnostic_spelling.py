#!/usr/bin/env python3
"""A diagnostic spells a type the way its reader could have written it.

`Type::name` is the SCHEMA KEY — `typedef.rs` builds wrapper types from it and `state`
looks stores up by it, so it renders a keyed collection in the notation the compiler
identifies it by: `hash<It,["k"]>` for the `hash<It[k]>` the author wrote.  `Type::source_name`
is the user-facing spelling.  A message that asks the first one names a type that does not
exist in the reader's program.

Nothing fails when a message is merely unreadable: the compiler is right, the program is
wrong, and only the author pays.  So the drift is invisible and it returned three times
(loft#956, loft#1434, loft#1445), each pass converting the sites someone happened to have a
symptom for.  This gate is the structural answer — one check, so forgetting is a red build
rather than a fourth residue.

Usage:
    diagnostic_spelling.py check    # exit 1 on any unmarked `Type::name` in a diagnostic
    diagnostic_spelling.py list     # every site, allowed ones included, counted by file

A message that genuinely means the schema key marks its line `// schema-key`.
"""

import bisect
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src"

# What puts text in front of a user.  `diagnostic!` and `diagnostic_at!` are the two macro
# emitters and `specific!` routes one through a parse result; all three expand to
# `diagnostic_format`, which a dozen sites also call DIRECTLY to build a message they hand to
# `lexer.diagnostic(…)` themselves.  Anchoring on the macros alone missed those — the
# `(N-Store)` warning is one, and its corpus pin kept the schema spelling through a sweep
# that reported itself complete.  A render outside all four — an IR dump, a store key,
# `Type::show` — is the compiler talking to itself and is left alone.
EMITTER_RE = re.compile(r"\b(?:diagnostic|diagnostic_at|specific)!\s*\(|\bdiagnostic_format\s*\(")

# `Type::name(data)` takes the `Data` it renders against.  Every other `name` in the tree
# takes something else — `Definition::name()` nothing, `Variables::name(var_nr)` an index,
# `Data::name("integer")` a string — so the ARGUMENT is what identifies this call, not the
# method name.  Listing the `Data` spellings keeps the gate precise; a new spelling makes the
# gate silent rather than wrong, and `cargo check` catches a conversion that was never a
# `Type` (`source_name` exists only on `Type`).
NAME_CALL_RE = re.compile(r"\.name\(\s*&?(?:self\.)?data\s*\)")

ESCAPE = "// schema-key"

# The layers that talk to the author.  Inside them a `Type::name` render is presumed
# user-facing and must say so; everywhere else (`state`, `generation`, `typedef`) the schema
# key is the normal answer and only a render inside an emitter span is suspect.
#
# The presumption is not a convenience — it is what the span rule alone cannot do.  Measured
# on the sweep this gate was written for: of the renders left in these two layers after every
# in-span site had been converted, 20 of 26 were still user-facing, each one a `let name =
# tp.name(data);` on the line ABOVE a `diagnostic!` that interpolates it.  A span cannot see
# those, and neither can it see a helper called from inside one (`cure_spelling`, whose whole
# job is to spell a cure).  One message had BOTH spellings in it, three lines apart.
DIAGNOSTIC_LAYERS = ("src/parser/", "src/variables/")


def spans(text):
    """Yield (start, end) offsets of each diagnostic-emitter invocation.

    Scans for the opening `macro!(` and walks to its matching `)`, skipping over string and
    char literals and comments so that a `)` inside a message never ends a span early.
    Nested invocations are covered by their enclosing span, which is what the caller wants:
    an argument built inside a diagnostic is still rendered to the same reader.
    """
    for m in EMITTER_RE.finditer(text):
        i = m.end()
        depth = 1
        while i < len(text) and depth:
            c = text[i]
            if c == "\\":
                i += 2
                continue
            if c == '"':
                i += 1
                while i < len(text):
                    if text[i] == "\\":
                        i += 2
                        continue
                    if text[i] == '"':
                        break
                    i += 1
            elif c == "'" and text.startswith("'\\", i):
                i += 2
                while i < len(text) and text[i] != "'":
                    i += 1
            elif text.startswith("//", i):
                nl = text.find("\n", i)
                i = len(text) if nl < 0 else nl
                continue
            elif text.startswith("/*", i):
                end = text.find("*/", i)
                i = len(text) if end < 0 else end + 1
            elif c == "(":
                depth += 1
            elif c == ")":
                depth -= 1
            i += 1
        yield m.start(), i


def sites():
    """Every `Type::name` render this gate governs, as (path, line, marked, text)."""
    found = []
    for path in sorted(SRC.rglob("*.rs")):
        rel = str(path.relative_to(ROOT))
        in_layer = rel.startswith(DIAGNOSTIC_LAYERS)
        text = path.read_text(encoding="utf-8", errors="replace")
        lines = text.split("\n")
        # Offset -> line number, computed once per file.
        offsets = []
        pos = 0
        for ln in lines:
            offsets.append(pos)
            pos += len(ln) + 1
        if in_layer:
            hits = [m.start() for m in NAME_CALL_RE.finditer(text)]
        else:
            hits = [
                m.start()
                for start, end in spans(text)
                for m in NAME_CALL_RE.finditer(text, start, end)
            ]
        for hit in sorted(set(hits)):
            lineno = bisect.bisect_right(offsets, hit)
            line = lines[lineno - 1]
            found.append((path.relative_to(ROOT), lineno, ESCAPE in line, line.strip()))
    return found


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "check"
    if mode not in ("check", "list"):
        print(__doc__)
        return 2
    all_sites = sites()
    bad = [s for s in all_sites if not s[2]]

    if mode == "list":
        by_file = {}
        for path, lineno, marked, line in all_sites:
            by_file.setdefault(str(path), []).append((lineno, marked, line))
        for path in sorted(by_file, key=lambda p: -len(by_file[p])):
            rows = by_file[path]
            print(f"{path}: {len(rows)}")
            for lineno, marked, line in rows:
                print(f"  {lineno}{' [schema-key]' if marked else ''}: {line[:110]}")
        print(f"\ntotal {len(all_sites)}, marked {len(all_sites) - len(bad)}, unmarked {len(bad)}")
        return 0

    if not bad:
        print(f"diagnostic spelling: {len(all_sites)} render(s) in diagnostics, all source_name")
        return 0
    print(
        f"{len(bad)} diagnostic(s) render a type with `Type::name` — the SCHEMA KEY, which\n"
        "spells a keyed collection `hash<It,[\"k\"]>` rather than the `hash<It[k]>` its reader\n"
        "wrote.  Use `Type::source_name`, or mark the line `// schema-key` if the message\n"
        "genuinely names the key:\n"
    )
    for path, lineno, _marked, line in bad:
        print(f"  {path}:{lineno}: {line[:110]}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
