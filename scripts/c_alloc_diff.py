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
counts, on stores made after it, each named by the order it first appears, and each shown with
the source line it ran at (the `@ <line>` lines both traces write when the line changes): the interpreter's
value stack, its constant store and the stores it builds constants in are its own setup, and
so are a par worker's stack and constant store, which the trace marks `S`.  A program without
`main` is skipped.

A rewrite the C writer applies and the interpreter does not (`C_ONLY_REWRITES`) may ask for
other room or keep a value out of the store: `(R-PushFill)` reserves a loop's pushes before the
loop, where the interpreter grows push by push; `(R-Transient)` builds a value that is read and
dropped outside the store.  Room is capacity and observably nothing (`(Col-Capacity)`), so such a difference
is the rewrite's, not a divergence of the store model (@C122; STORE_MODEL.md); `--attribute`
names it, and what no switch accounts for is the allocator's to explain.

Inside a `par` region the comparison is approximate: the workers' events interleave in the
trace, and each interpreter worker numbers its stores on its own, so the order there is not
the program's."""
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
    ("R-Transient", "LOFT_NO_TRANSIENT"),
    ("R-DecodeView", "LOFT_NO_DECODE_VIEW"),
    ("R-DecodeView/structure", "LOFT_NO_DECODE_STRUCTURE"),
    ("R-Destination", "LOFT_NO_DESTINATION"),
    ("R-RangedCall", "LOFT_NO_RANGED_CALLS"),
]

class Event(str):
    """One event line, compared as its text, with the source line it ran at (`@`, `?` unknown)."""
    def __new__(cls, text, line):
        e = super().__new__(cls, text)
        e.line = line
        return e

def program_events(lines):
    """The events after the program starts (`M`), on stores made after it, each store named by
    the order it first appears.  A store the interpreter marks as its own (`S`: a state's value
    stack and constant store, which a par worker makes too) is left out — a C unit has neither."""
    rows = [l.split() for l in lines]
    # A serial can name more than one store over a run (each par worker counts its own), so an
    # `S` marks the latest store made under its serial that no `S` marked yet — two workers'
    # stacks, made and marked interleaved, are both marked — and the mark covers its `N` and
    # every event up to the next `N` of that serial.
    own, made = set(), {}
    for i, f in enumerate(rows):
        if len(f) >= 2 and f[0] == "N":
            made.setdefault(f[1], []).append(i)
        elif len(f) == 2 and f[0] == "S" and made.get(f[1]):
            own.add(made[f[1]].pop())
    out, names, started, dropped, at = [], {}, False, set(), "?"
    for i, f in enumerate(rows):
        if not f:
            continue
        if f[0] == "@":
            at = f[1] if len(f) > 1 else "?"
            continue
        if f[0] == "M":
            started = True
            continue
        if len(f) >= 2 and f[0] == "N":
            if i in own:
                dropped.add(f[1])
            else:
                dropped.discard(f[1])
        if not started or len(f) < 2 or f[0] == "S" or f[1] in dropped:
            continue
        if f[0] == "N":
            names[f[1]] = str(len(names) + 1)
        if f[1] not in names:
            continue
        f[1] = names[f[1]]
        out.append(Event(" ".join(f), at))
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
    # `LOFT_C_DEFINES` adds the C runtime's compile-time switches (`-DLOFT_NO_KEEP_ROOM`, …), the
    # policies a C unit fixes when it is built rather than when it runs.
    defines = env.get("LOFT_C_DEFINES", "").split()
    r = subprocess.run(["cc", "-std=c11", "-O1", "-DLOFT_ALLOC_TRACE", *defines, src, "-o", exe, "-lm", *links],
                       capture_output=True, text=True)
    if r.returncode != 0:
        return None, "cc: " + r.stderr[:300]
    # Run where the interpreter runs: a program that opens a file by a relative path reads the
    # same file on both sides.
    subprocess.run([exe], env={**env, "LOFT_ALLOC_TRACE": t}, stdout=subprocess.DEVNULL,
                   stderr=subprocess.DEVNULL, timeout=120)
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
            a = f"{want[k]:<18} @{want[k].line}" if k < len(want) else "-"
            b = f"{got[k]:<18} @{got[k].line}" if k < len(got) else "-"
            print(f"   {'>' if k == i else ' '} {a:<28} {b}")
        if attr:
            who = attribute(prog)
            print(f"    the rewrite's: {who[0]} (agrees under {who[1]})" if who
                  else "    no C-only rewrite accounts for it")

if __name__ == "__main__":
    sys.exit(main())
