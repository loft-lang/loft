#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The C backend's store allocator events against the interpreter's (@PLN200 B5).

    scripts/c_alloc_diff.py FILE...     # each program: `same (N events)`, or the first event
                                        # that differs with its context, or why it was skipped
    scripts/c_alloc_diff.py --attribute FILE...
                                        # a program that differs is run again with each C-only
                                        # rewrite switched off: one under which it agrees is
                                        # that rewrite's difference, not the allocator's

The memory model is loft's: a C program claims, deletes, resizes, makes and frees stores
exactly as the interpreter does, so every record sits where the interpreter puts it.  Both
write `LOFT_ALLOC_TRACE` — the interpreter from the `op-census` build (`cargo build --release
--lib --bin loft --features op-census --target-dir target/op-census`, the default `LOFT_BIN`),
the C built with `-DLOFT_ALLOC_TRACE`.  Only what follows the `M` line (the program starts)
counts, on stores made after it, each named by the order it first appears: the interpreter's
value stack, its constant store and the stores it builds constants in are its own setup.  A
program without `main` is skipped.

A rewrite the C writer applies and the interpreter does not (`C_ONLY_REWRITES`) may ask for
other room: `(R-PushFill)` reserves a loop's pushes before the loop, where the interpreter grows
push by push.  Room is capacity and observably nothing (`(Col-Capacity)`), so such a difference
is the rewrite's, not a divergence of the store model (@C122; STORE_MODEL.md); `--attribute`
names it, and what no switch accounts for is the allocator's to explain."""
import os, re, subprocess, sys, tempfile

LOFT = os.environ.get("LOFT_BIN", "target/op-census/release/loft")

# The rewrites the C writer applies and the interpreter does not, each with the switch that
# turns it off on both, in the order `--attribute` tries them.
C_ONLY_REWRITES = [
    ("R-PushFill", "LOFT_NO_PUSH_FILL"),
    ("R-PushFill/run", "LOFT_NO_PUSH_RUN_RESERVE"),
    ("R-Apart", "LOFT_NO_APART"),
    ("R-ValueRecord", "LOFT_NO_VALUE_RECORD"),
    ("R-ValueLocal", "LOFT_NO_VALUE_LOCAL"),
    ("R-AppendTwin", "LOFT_NO_APPEND_TWIN"),
]

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

def interp(prog, d, env):
    t = os.path.join(d, "i.txt")
    subprocess.run([LOFT, "--interpret", prog], env={**env, "LOFT_ALLOC_TRACE": t},
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120)
    return program_events(open(t, encoding="utf-8"))

def c(prog, d, env):
    src, exe, t = (os.path.join(d, x) for x in ("p.c", "p", "c.txt"))
    r = subprocess.run([LOFT, "--emit-c", src, prog], capture_output=True, text=True, timeout=120, env=env)
    if r.returncode != 0:
        return None, "refused: " + r.stderr.strip().splitlines()[-1] if r.stderr.strip() else "refused"
    # The archives the unit names (`/* link: … */`: a `[c] shim`, the TLS client of a unit that
    # fetches over a URL or pages an image), as c_sweep.py links them.
    with open(src, encoding="utf-8", errors="replace") as f:
        links = re.findall(r"^/\* link: (.*) \*/$", f.read(), re.M)
    r = subprocess.run(["cc", "-std=c11", "-O1", "-DLOFT_ALLOC_TRACE", src, "-o", exe, "-lm", *links],
                       capture_output=True, text=True)
    if r.returncode != 0:
        return None, "cc: " + r.stderr[:300]
    subprocess.run([exe], env={**env, "LOFT_ALLOC_TRACE": t}, stdout=subprocess.DEVNULL,
                   stderr=subprocess.DEVNULL, timeout=120, cwd=d)
    return program_events(open(t, encoding="utf-8")), None

def compare(prog, env):
    """`(None, why)` when the program was not compared, else `(i, want, got)` with `i` the
    first differing event, `None` when the two agree."""
    with tempfile.TemporaryDirectory() as d:
        try:
            got, why = c(prog, d, env)
            want = interp(prog, d, env) if got is not None else None
        except Exception as e:
            return None, f"error {type(e).__name__}"
        if got is None:
            return None, why
    i = next((k for k, (a, b) in enumerate(zip(want, got)) if a != b), None)
    if i is None and len(want) != len(got):
        i = min(len(want), len(got))
    return i, want, got

def attribute(prog):
    """The C-only rewrite under whose switch the program agrees, or None."""
    for rule, switch in C_ONLY_REWRITES:
        r = compare(prog, {**os.environ, switch: "1"})
        if r[0] is None and len(r) == 3:
            return rule, f"{switch}=1"
    # A program can carry more than one: then only all of them off together agree.
    r = compare(prog, {**os.environ, **{sw: "1" for _, sw in C_ONLY_REWRITES}})
    if r[0] is None and len(r) == 3:
        return (" + ".join(rule for rule, _ in C_ONLY_REWRITES),
                " ".join(f"{sw}=1" for _, sw in C_ONLY_REWRITES))
    return None

def main():
    args = sys.argv[1:]
    if not args or any(a in ("-h", "--help") for a in args):
        print(__doc__)
        return 0
    attr = "--attribute" in args
    for prog in (a for a in args if a != "--attribute"):
        if not re.search(r"^\s*(pub\s+)?fn\s+main\s*\(", open(prog, encoding="utf-8", errors="replace").read(), re.M):
            continue
        r = compare(prog, dict(os.environ))
        if len(r) == 2:
            print(f"{prog}: {r[1]}")
            continue
        i, want, got = r
        if i is None:
            print(f"{prog}: same ({len(want)} events)")
            continue
        print(f"{prog}: differs at event {i} of {len(want)} / {len(got)}")
        for k in range(max(0, i - 4), i + 4):
            a = want[k] if k < len(want) else "-"
            b = got[k] if k < len(got) else "-"
            print(f"   {'>' if k == i else ' '} {a:<28} {b}")
        if attr:
            who = attribute(prog)
            print(f"    the rewrite's: {who[0]} (agrees under {who[1]})" if who
                  else "    no C-only rewrite accounts for it")

if __name__ == "__main__":
    sys.exit(main())
