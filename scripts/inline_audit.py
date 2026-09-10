#!/usr/bin/env python3
"""Which `#[inline]` functions did rustc DECLINE to inline?

A generic `#[inline]` function is visible to the consumer's crate, so rustc decides on
SIZE.  When a hot fast path shares a body with a cold half — an error report, a growth
ladder, an out-of-range fallback — the whole body is what gets measured, the decision is
lost, and every call pays a call for a fast path that is a compare and a load.

That shape has been found three times in loft (`get_elem_hoisted`, its write-side twin
`vec_set_hoisted_or_raise_runtime`, and `watch_oob_text`), each time by reading a profile
and recognising it.  This makes the CLASS visible instead: an `#[inline]` function that
still has an out-of-line symbol in an optimised binary did not inline somewhere.

    scripts/inline_audit.py <optimised-binary> [src-dir]

⚠ It REPORTS; it is not a gate, and its output is leads rather than a verdict.  rustc
routinely emits an out-of-line copy of a function it also inlines at most call sites, so
a name here is not by itself a defect.  Cross it with a PROFILE: a row that is marked
`#[inline]`, appears as a symbol, AND holds measurable self time is the candidate worth
opening.  Common short names (`new`, `get`, `drop`, `insert`) over-match on purpose —
narrowing them would hide the real ones behind a cleverer matcher.

Build the binary with `-C debuginfo=1` and NO `-Cstrip=symbols`, or it resolves nothing
and the empty report reads exactly like a clean bill of health.
"""

import collections
import pathlib
import re
import subprocess
import sys


def marked_inline(srcdir: str) -> dict[str, str]:
    """Every `fn` carrying a bare `#[inline]`, mapped to where it is written.

    Not `#[inline(always)]` (already forced) nor `#[inline(never)]` (already the cure).
    """
    found: dict[str, str] = {}
    for path in pathlib.Path(srcdir).rglob("*.rs"):
        lines = path.read_text(errors="replace").splitlines()
        for i, line in enumerate(lines):
            if line.strip() != "#[inline]":
                continue
            # The fn may sit under further attributes or doc lines.
            for j in range(i + 1, min(i + 12, len(lines))):
                m = re.search(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)", lines[j])
                if m:
                    found[m.group(1)] = f"{path}:{j + 1}"
                    break
                stripped = lines[j].strip()
                if stripped.startswith("#[") or stripped.startswith("///") or not stripped:
                    continue
                break
    return found


def out_of_line(binary: str) -> collections.Counter:
    """Every function name the optimised binary kept a real symbol for."""
    out = subprocess.run(
        ["nm", "-C", "--defined-only", binary], capture_output=True, text=True
    ).stdout
    seen: collections.Counter = collections.Counter()
    for line in out.splitlines():
        parts = line.split(" ", 2)
        if len(parts) < 3:
            continue
        m = re.search(r"(?:^|::)([a-z_][A-Za-z0-9_]*)(?:::<|\(|$)", parts[2])
        if m:
            seen[m.group(1)] += 1
    return seen


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    binary = sys.argv[1]
    srcdir = sys.argv[2] if len(sys.argv) > 2 else "src"
    marked = marked_inline(srcdir)
    present = out_of_line(binary)
    if not present:
        print(f"no symbols resolved in {binary} — is it stripped? see the note above")
        return 1
    hits = sorted(
        ((n, present[n], loc) for n, loc in marked.items() if present.get(n)),
        key=lambda t: -t[1],
    )
    print(
        f"{len(marked)} functions marked #[inline]; "
        f"{len(hits)} still have an out-of-line symbol in {binary}\n"
    )
    print(f"{'copies':>6}  {'function':40s} where")
    for name, copies, loc in hits:
        print(f"{copies:>6}  {name:40s} {loc}")
    print("\nLeads, not verdicts — cross with a profile; see this file's header.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
