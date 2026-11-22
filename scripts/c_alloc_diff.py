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

A difference of ORDER alone is named as such (`order only:`): each store's own events alike
with separate stores interleaving otherwise, or a release walk's records the same in another
order.  The store model is per store, and neither order is part of it.

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
    """One event line, compared as its text, with the source line it ran at (`@`, `?` unknown)
    and whether it ran inside a `par` region (between the trace's `P` and `Q`)."""
    def __new__(cls, text, line, par=False):
        e = super().__new__(cls, text)
        e.line = line
        e.par = par
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
    out, names, started, dropped, at, made, par = [], {}, False, set(), "?", 0, 0
    for i, f in enumerate(rows):
        if not f:
            continue
        if f[0] == "@":
            at = f[1] if len(f) > 1 else "?"
            continue
        if f[0] in ("P", "Q") and len(f) == 1:
            par = par + 1 if f[0] == "P" else max(par - 1, 0)
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
            # A counter, not the table's size: a number made again (a freed slot reused)
            # replaces its key without growing the table, and two stores took one name.
            made += 1
            names[f[1]] = str(made)
        if f[1] not in names:
            continue
        f[1] = names[f[1]]
        out.append(Event(" ".join(f), at, par > 0))
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

def per_store(events):
    """Each store's own events in order, the stores taken as a multiset: what a program's
    stores went through, whatever order separate stores' events interleave in."""
    seq = {}
    for e in events:
        f = str(e).split()
        seq.setdefault(f[1], []).append(" ".join([f[0]] + f[2:]))
    return sorted(seq.values())

def runs_as_sets(events):
    """The events with each run of one kind (a release walk's deletes, a scope exit's frees)
    taken as a set: the same records released, in any order."""
    out, cur = [], []
    for e in events:
        if cur and cur[0].split()[0] != str(e).split()[0]:
            out += sorted(cur)
            cur = []
        cur.append(str(e))
    return out + sorted(cur)

def no_capacity(events):
    """The events with a made store's starting capacity left out (`N <store>`): room a store
    starts with places no record, and a slot made again keeps the buffer it had (a par
    worker's stack slot, 1024 words, on the interpreter)."""
    return [" ".join(str(e).split()[:2]) if str(e).startswith("N ") else str(e) for e in events]

def region_only(want, got):
    """STORE_MODEL D46: a `par` region's machinery is each runtime's own — the events between its
    `P` and `Q`, and where a store alive across the region ends (the interpreter hands a
    worker's stores to the parent at the join and frees them there, the C in the worker).
    Answers whether everything else is alike: every event before the first region, event for
    event, and every store made after one, per store."""
    def split(ev):
        first = next((k for k, e in enumerate(ev) if e.par), None)
        if first is None:
            return None
        last = max(k for k, e in enumerate(ev) if e.par)
        before = ev[:first]
        crossing = {e.split()[1] for e in ev[:last + 1]}
        after = [e for e in ev[last + 1:] if e.split()[1] not in crossing]
        return before, after
    w, g = split(want), split(got)
    if w is None or g is None:
        return False
    return w[0] == g[0] and per_store(w[1]) == per_store(g[1])

def order_only(want, got):
    """Why two traces that differ by position still describe the same stores, or None: each
    store's own events alike (separate stores interleave in the order a program's expressions
    run), each run of one kind alike as a set (a release walk's order), or everything outside
    the `par` regions alike (D46); a made store's starting capacity aside."""
    want = [Event(t, e.line, e.par) for t, e in zip(no_capacity(want), want)]
    got = [Event(t, e.line, e.par) for t, e in zip(no_capacity(got), got)]
    if len(want) == len(got):
        if want == got:
            return "the same events, a store's starting capacity aside"
        if runs_as_sets(want) == runs_as_sets(got):
            return "the same records in each run, in another order (a release walk's order)"
        if per_store(want) == per_store(got):
            return "the same events per store, interleaved in another order"
    if region_only(want, got):
        return ("the same before its par region and in every store made after it; inside, and "
                "where a store alive across it ends, each runtime's par machinery (D46)")
    return None

def agrees(r):
    """A comparison that agrees: event for event, or by order only."""
    return len(r) == 3 and (r[0] is None or order_only(r[1], r[2]) is not None)

def attribute(prog):
    """The C-only rewrite under whose switch the program agrees (event for event or by order
    only), or None."""
    for rule, switch in C_ONLY_REWRITES:
        if agrees(compare(prog, {**os.environ, switch: "1"})):
            return rule, f"{switch}=1"
    # A program can carry more than one: then only all of them off together agree.
    if agrees(compare(prog, {**os.environ, **{sw: "1" for _, sw in C_ONLY_REWRITES}})):
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
        why = order_only(want, got)
        if why:
            print(f"    order only: {why}")
            continue
        if attr:
            who = attribute(prog)
            print(f"    the rewrite's: {who[0]} (agrees under {who[1]})" if who
                  else "    no C-only rewrite accounts for it")

if __name__ == "__main__":
    sys.exit(main())
