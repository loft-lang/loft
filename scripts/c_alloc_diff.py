#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The C backend's store allocator events against the interpreter's (@PLN200 B5).

    scripts/c_alloc_diff.py FILE...     # each program: `same (N events)`, or the first event
                                        # that differs with its context, or why it was skipped
    scripts/c_alloc_diff.py --self-test # the comparison's verdicts on hand-made traces
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
trace by the threads' timing, so the order there is not the program's.  Each worker numbers its
stores on its own, and a store is named `<worker>.<serial>` by the worker that made it."""
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
    return program_walk(lines)[0]

def program_names(lines):
    """Each store's trace name → the name `program_events` gives it: the latest store under that
    trace name, for a reader of the stores as a run leaves them (`store_diff.py`)."""
    return program_walk(lines)[1]

def program_walk(lines):
    """`program_events`, and the map from a trace name to the name it was given."""
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
    return out, names

def interp(prog, d, env):
    t = os.path.join(d, "i.txt")
    subprocess.run([LOFT, "--interpret", prog], env={**env, "LOFT_ALLOC_TRACE": t},
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120)
    return program_events(open(t, encoding="utf-8"))

def c(prog, d, env):
    src, exe, t = (os.path.join(d, x) for x in ("p.c", "p", "c.txt"))
    # Every statement records its line, so each event carries the line that made it.
    r = subprocess.run([LOFT, "--emit-c", src, prog], capture_output=True, text=True, timeout=120,
                       env={**env, "LOFT_C_TRACE_SITES": "1"})
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
    """Each store's own events in order, by the store's name: what each store went through,
    whatever order separate stores' events interleave in.  The names come from the order the
    stores were made (`program_events`), so a record that moved to another store differs."""
    seq = {}
    for e in events:
        f = str(e).split()
        seq.setdefault(f[1], []).append(" ".join([f[0]] + f[2:]))
    return seq

def runs_as_sets(events):
    """The events with each run of releases — a release walk's deletes, a scope exit's frees —
    taken as a set: the same records released, in any order.  Claims keep their order: where a
    record lands is part of the model."""
    out, cur = [], []
    for e in events:
        k = str(e).split()[0]
        if cur and cur[0].split()[0] != k:
            out += sorted(cur) if cur[0].split()[0] in ("D", "F") else cur
            cur = []
        cur.append(str(e))
    return out + (sorted(cur) if cur and cur[0].split()[0] in ("D", "F") else cur)

def no_capacity(events):
    """The events with a made store's starting capacity left out (`N <store>`): room a store
    starts with places no record, and a slot made again keeps the buffer it had (a par
    worker's stack slot, 1024 words, on the interpreter)."""
    return [" ".join(str(e).split()[:2]) if str(e).startswith("N ") else str(e) for e in events]

def region_only(want, got):
    """STORE_MODEL D46: inside a `par` region the workers' events interleave by the threads'
    timing — the events between its `P` and `Q`, and WHERE a store the region touches ends, are
    not compared event by event.  Everything
    else is held to the model: every event outside the regions, per store (each store named
    by its first appearance outside them, so a store made inside a region and used after it
    is compared too), with only a touched store's free exempt from its place — and that free
    must occur as often on both sides, at most once (no leak, no double free)."""
    def outer(ev):
        names, seq, frees, touched = {}, {}, {}, set()
        for e in ev:
            f = str(e).split()
            if f[0] == "F":
                frees[f[1]] = frees.get(f[1], 0) + 1
            if e.par:
                touched.add(f[1])
                continue
            if f[1] not in names:
                names[f[1]] = str(len(names) + 1)
            seq.setdefault(names[f[1]], []).append(" ".join([f[0]] + f[2:]))
        return names, seq, frees, touched
    wn, ws, wf, wt = outer(want)
    gn, gs, gf, gt = outer(got)
    # Both sides ran the region: one without it is no machinery difference.
    if not (wt and gt):
        return False
    # The stores a region touched, on either side, by their outer names.
    exempt = {wn[s] for s in wt if s in wn} | {gn[s] for s in gt if s in gn}
    def strip(seq):
        return {k: [x for x in v if not (k in exempt and x == "F")] for k, v in seq.items()}
    if strip(ws) != strip(gs):
        return False
    wfree = {wn[s]: c for s, c in wf.items() if s in wn}
    gfree = {gn[s]: c for s, c in gf.items() if s in gn}
    return all(wfree.get(k, 0) == gfree.get(k, 0) <= 1 for k in exempt)

def order_only(want, got):
    """Why two traces that differ by position still describe the same stores, or None: each
    store's own events alike (separate stores interleave in the order a program's expressions
    run), each run of releases alike as a set (a release walk's order), or everything outside
    the `par` regions alike (D46); a made store's starting capacity aside."""
    want = [Event(t, e.line, e.par) for t, e in zip(no_capacity(want), want)]
    got = [Event(t, e.line, e.par) for t, e in zip(no_capacity(got), got)]
    if len(want) == len(got) and not any(e.par for e in want + got):
        if want == got:
            return "the same events, a store's starting capacity aside"
        if runs_as_sets(want) == runs_as_sets(got):
            return "the same records in each run of releases, in another order"
        # A store's number is only the order it was made in, which two backends may take
        # differently (`981-split-ownership-return`): the stores match when each one's whole
        # history has its twin on the other side — the per-store model's equivalence.
        if sorted(per_store(want).values()) == sorted(per_store(got).values()):
            return "the same stores, each with the same events, made or interleaved in another order"
    if region_only(want, got):
        return ("the same outside its par regions, store by store; inside them, and where a "
                "store they touch is freed, the threads' timing (D46)")
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

# The cases the comparison's verdict is held to (`--self-test`): a pair of event traces (`;`
# between events, `P`/`Q` a par region) and whether `order_only` may accept them.  The rejects
# are an adversarial review's holes in an earlier region check (STORE_MODEL D46): work between
# two regions, a store claimed differently or leaked or freed twice after one, a truncated
# trace, claims whose places differ.
SELF_TEST = [
    ("serial work between two regions differs", False,
     "N 5 8;P;C 5 4 0;Q;C 5 4 4;N 6 8;C 6 2 0;F 6;P;C 5 2 8;Q;F 5",
     "N 5 8;P;C 5 4 0;Q;C 5 9 4;N 6 8;C 6 7 0;P;C 5 2 8;Q;F 5"),
    ("a store from before a region claimed differently after it", False,
     "N 5 8;C 5 4 0;P;N 7 8;Q;C 5 4 4;D 5 0;F 5", "N 5 8;C 5 4 0;P;N 7 8;Q;C 5 6 20;F 5"),
    ("a store alive across a region never freed on one side", False,
     "N 5 8;C 5 4 0;P;N 7 8;Q;F 5", "N 5 8;C 5 4 0;P;N 7 8;Q"),
    ("a trace that stops after the region", False,
     "N 5 8;C 5 4 0;P;N 7 8;Q;C 5 4 4;D 5 0;D 5 4;F 5", "N 5 8;C 5 4 0;P;N 7 8;Q"),
    ("a store freed twice after the region", False,
     "N 5 8;P;N 7 8;Q;F 5", "N 5 8;P;N 7 8;Q;F 5;F 5"),
    ("a store made in the region, used differently after it", False,
     "P;N 5 8;Q;C 5 4 0;C 5 4 4;F 5", "P;N 5 8;Q;C 5 4 8;F 5"),
    ("an extra store after the region", False,
     "N 5 8;P;N 7 8;Q;F 5", "N 5 8;P;N 7 8;Q;N 9 8;F 9;F 5"),
    ("a difference before the region", False,
     "N 5 8;C 5 4 0;P;C 5 1 4;Q;F 5", "N 5 8;C 5 4 8;P;C 5 1 4;Q;F 5"),
    ("two claims of one store in each other's places", False,
     "N 5 8;C 5 4 8;C 5 4 0;F 5", "N 5 8;C 5 4 0;C 5 4 8;F 5"),
    ("an extra store inside one side's region", True,
     "N 5 8;P;N 7 8;Q;N 9 8;C 9 4 0;F 9;F 5", "N 5 8;P;N 7 8;N 8 8;Q;N 9 8;C 9 4 0;F 9;F 5"),
    ("a store's starting capacity alone", True, "N 5 8;C 5 4 0;F 5", "N 5 99;C 5 4 0;F 5"),
    ("a release walk's order", True,
     "N 5 8;C 5 4 0;C 5 4 4;D 5 0;D 5 4;F 5", "N 5 8;C 5 4 0;C 5 4 4;D 5 4;D 5 0;F 5"),
    ("two stores made in the other order", True,
     "N 5 8;N 6 8;C 5 4 0;C 6 8 0;F 5;F 6", "N 5 8;N 6 8;C 5 8 0;C 6 4 0;F 6;F 5"),
    ("where a store the region touches is freed", True,
     "N 5 8;C 5 4 0;P;N 7 8;C 5 2 4;Q;F 5", "N 5 8;C 5 4 0;P;N 7 8;C 5 2 4;F 5;Q"),
]

def self_test():
    """Exit status 1 when `order_only` gives any `SELF_TEST` case the wrong verdict."""
    def ev(s):
        return program_events(["M"] + [x.strip() for x in s.split(";")])
    bad = 0
    for name, accept, a, b in SELF_TEST:
        got = order_only(ev(a), ev(b)) is not None
        if got != accept:
            bad += 1
            print(f"wrong verdict: {name}: {'accepted' if got else 'rejected'}")
    print(f"{len(SELF_TEST) - bad} of {len(SELF_TEST)} verdicts right")
    return 1 if bad else 0

def main():
    args = sys.argv[1:]
    if args == ["--self-test"]:
        return self_test()
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
