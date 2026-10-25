#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The C backend's store allocator events against the interpreter's (@PLN200 B5).

    scripts/c_alloc_diff.py FILE...     # each program: `same (N events)`, or the first event
                                        # that differs with its context, or why it was skipped

The memory model is loft's: a C program claims, deletes, resizes, makes and frees stores
exactly as the interpreter does, so every record sits where the interpreter puts it.  Both
write `LOFT_ALLOC_TRACE` — the interpreter from the `op-census` build (`cargo build --release
--lib --bin loft --features op-census --target-dir target/op-census`, the default `LOFT_BIN`),
the C built with `-DLOFT_ALLOC_TRACE`.  Only what follows the `M` line (the program starts)
counts, on stores made after it, each named by the order it first appears: the interpreter's
value stack, its constant store and the stores it builds constants in are its own setup.  A
program without `main` is skipped."""
import os, re, subprocess, sys, tempfile

LOFT = os.environ.get("LOFT_BIN", "target/op-census/release/loft")

def program_events(lines):
    """The events after the program starts (`M`), on stores made after it, each store named by
    the order it first appears."""
    out, names, started = [], {}, False
    for line in lines:
        f = line.split()
        if not f:
            continue
        if f[0] == "M":
            started = True
            continue
        if not started or len(f) < 2:
            continue
        if f[0] == "N":
            names[f[1]] = str(len(names) + 1)
        if f[1] not in names:
            continue
        f[1] = names[f[1]]
        out.append(" ".join(f))
    return out

def interp(prog, d):
    t = os.path.join(d, "i.txt")
    subprocess.run([LOFT, "--interpret", prog], env={**os.environ, "LOFT_ALLOC_TRACE": t},
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120)
    return program_events(open(t, encoding="utf-8"))

def c(prog, d):
    src, exe, t = (os.path.join(d, x) for x in ("p.c", "p", "c.txt"))
    r = subprocess.run([LOFT, "--emit-c", src, prog], capture_output=True, text=True, timeout=120)
    if r.returncode != 0:
        return None, "refused: " + r.stderr.strip().splitlines()[-1] if r.stderr.strip() else "refused"
    r = subprocess.run(["cc", "-std=c99", "-O1", "-DLOFT_ALLOC_TRACE", src, "-o", exe, "-lm"],
                       capture_output=True, text=True)
    if r.returncode != 0:
        return None, "cc: " + r.stderr[:300]
    subprocess.run([exe], env={**os.environ, "LOFT_ALLOC_TRACE": t}, stdout=subprocess.DEVNULL,
                   stderr=subprocess.DEVNULL, timeout=120, cwd=d)
    return program_events(open(t, encoding="utf-8")), None

def main():
    if len(sys.argv) < 2 or any(a in ("-h", "--help") for a in sys.argv[1:]):
        print(__doc__)
        return 0
    for prog in sys.argv[1:]:
        if not re.search(r"^\s*(pub\s+)?fn\s+main\s*\(", open(prog, encoding="utf-8", errors="replace").read(), re.M):
            continue
        with tempfile.TemporaryDirectory() as d:
            try:
                got, why = c(prog, d)
                want = interp(prog, d) if got is not None else None
            except Exception as e:
                print(f"{prog}: error {type(e).__name__}")
                continue
            if got is None:
                print(f"{prog}: {why}")
                continue
            i = next((k for k, (a, b) in enumerate(zip(want, got)) if a != b), None)
            if i is None and len(want) == len(got):
                print(f"{prog}: same ({len(want)} events)")
                continue
            if i is None:
                i = min(len(want), len(got))
            print(f"{prog}: differs at event {i} of {len(want)} / {len(got)}")
            for k in range(max(0, i - 4), i + 4):
                a = want[k] if k < len(want) else "-"
                b = got[k] if k < len(got) else "-"
                print(f"   {'>' if k == i else ' '} {a:<28} {b}")

main()
