#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The stores the C backend leaves against the interpreter's, byte for byte (@PLN206 P4c).

    scripts/store_diff.py FILE...           # each program: `same (N stores, M records)`, or
                                            # every record that differs, by store, record and byte
    scripts/store_diff.py --attribute FILE  # a program that differs again with each C-only
                                            # rewrite off: one under which it agrees is that
                                            # rewrite's difference (`c_alloc_diff.py`'s list)
    scripts/store_diff.py --plant FILE      # the positive control: the C dump flips one byte
                                            # (`-DLOFT_STORE_DUMP_PLANT`), which must be reported

Both runtimes share one store layout (STORE_MODEL.md, P3), so the stores a run leaves can be
compared by their bytes.  Each side writes every live store at exit (`LOFT_STORE_DUMP`: the
interpreter's `src/store/dump.rs`, the C's `loft_store_dump`), and the allocation trace beside
it names each store by what happened to it (`c_alloc_diff.program_names`), so the two pair by
history, not by slot.  Both run under `LOFT_POISON_CLAIM`: a byte no one wrote reads the same
pattern on both, and a difference is a value one side wrote and the other did not.

A hash table is pseudo-random: both sides run with one fixed `LOFT_HASH_SEED`, so its seed and
its entries' places agree.  What still varies from run to run — the noise control runs the
interpreter twice — is named, not hidden: `vary from run to run`.

What a byte comparison cannot see — a store freed on one side and live on the other — is a
store missing from one dump, reported as such.  A program the C writer refuses, or that has no
`main`, is skipped and says why."""
import os, re, subprocess, sys, tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import c_alloc_diff as cad  # noqa: E402

LOFT = cad.LOFT


def trace_of(path):
    """The trace's name map and each store's making site: `({trace name: history name},
    {history name: line})` — the line the interpreter or the C wrote beside the store's `N`."""
    if not os.path.exists(path):
        return {}, {}
    lines = open(path, encoding="utf-8").read().splitlines()
    events, names = cad.program_walk(lines)
    sites = {}
    for e in events:
        f = str(e).split()
        if f[0] == "N":
            sites.setdefault(f[1], e.line)
    return names, sites


def read_dump(path, names):
    """`{history name: (header, {record: (words, hex)})}` for every store the trace names, and
    `"__slots"`: each slot number → the history name of the store in it (a stored reference
    names a store by its slot, which the two runtimes number differently)."""
    out, cur, slots = {}, None, {}
    out["__slots"] = slots
    if not os.path.exists(path):
        return out
    for line in open(path, encoding="utf-8"):
        f = line.split()
        if not f:
            continue
        if f[0] == "S":
            name = names.get(f[1])
            cur = None
            if name is not None:
                cur = out.setdefault(name, ((f[2], f[3]), {}))
                if len(f) > 4:
                    slots[int(f[4])] = name
        elif f[0] == "R" and cur is not None:
            cur[1][int(f[1])] = (int(f[2]), f[3] if len(f) > 3 else "")
    return out


def run_interp(prog, d, env):
    t, dump = os.path.join(d, "i.trace"), os.path.join(d, "i.dump")
    subprocess.run([LOFT, "--interpret", prog],
                   env={**env, "LOFT_ALLOC_TRACE": t, "LOFT_STORE_DUMP": dump, "LOFT_POISON_CLAIM": "1"},
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=300)
    names, sites = trace_of(t)
    return read_dump(dump, names), sites


def run_c(prog, d, env, plant):
    src, exe = os.path.join(d, "p.c"), os.path.join(d, "p")
    t, dump = os.path.join(d, "c.trace"), os.path.join(d, "c.dump")
    r = subprocess.run([LOFT, "--emit-c", src, prog], capture_output=True, text=True, timeout=300, env=env)
    if r.returncode != 0:
        last = r.stderr.strip().splitlines()[-1] if r.stderr.strip() else ""
        return None, "refused: " + last
    with open(src, encoding="utf-8", errors="replace") as f:
        links = re.findall(r"^/\* link: (.*) \*/$", f.read(), re.M)
    defines = ["-DLOFT_ALLOC_TRACE", "-DLOFT_STORE_DUMP", "-DLOFT_POISON_CLAIM"]
    if plant:
        defines.append("-DLOFT_STORE_DUMP_PLANT")
    defines += env.get("LOFT_C_DEFINES", "").split()
    r = subprocess.run(["cc", "-std=c11", "-O1", *defines, src, "-o", exe, "-lm", *links],
                       capture_output=True, text=True)
    if r.returncode != 0:
        return None, "cc: " + r.stderr[:300]
    subprocess.run([exe], env={**env, "LOFT_ALLOC_TRACE": t, "LOFT_STORE_DUMP": dump},
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=300)
    names, sites = trace_of(t)
    return (read_dump(dump, names), sites), None


def byte_runs(a, b):
    """Each run of differing bytes of two blocks (hex), as `(first, past, interp hex, c hex)`."""
    n = max(len(a), len(b)) // 2
    runs, lo = [], None
    for k in range(n + 1):
        differs = k < n and a[2 * k:2 * k + 2] != b[2 * k:2 * k + 2]
        if differs and lo is None:
            lo = k
        elif not differs and lo is not None:
            runs.append((lo, k, a[2 * lo:2 * k] or "--", b[2 * lo:2 * k] or "--"))
            lo = None
    return runs


def is_unwritten(h):
    """A run of the `LOFT_POISON_CLAIM` pattern (`efbeadde`, from any byte of it)."""
    pat = "efbeadde" * (len(h) // 8 + 2)
    return bool(h) and any(pat[k:k + len(h)] == h for k in range(0, 8, 2))


def is_zero(h):
    return bool(h) and set(h) == {"0"}


def same_store_refs(a, b, lo, hi, wslots, gslots):
    """Does every aligned 4-byte word the run `lo..hi` touches either agree, or hold on each
    side a slot number of the same store by history — one stored reference to one store?"""
    for w in range(lo - lo % 4, hi, 4):
        x, y = a[2 * w:2 * w + 8], b[2 * w:2 * w + 8]
        if x == y:
            continue
        if len(x) < 8 or len(y) < 8:
            return False
        sx, sy = int.from_bytes(bytes.fromhex(x), "little"), int.from_bytes(bytes.fromhex(y), "little")
        if not (sx in wslots and sy in gslots and wslots[sx] == gslots[sy]):
            return False
    return True


def differences(want, got, wsites=None, gsites=None, noise=None):
    """Every difference, as lines: a store one side lacks, a record one side lacks, a record of
    another size, and each run of differing bytes — each store named with the line it was made
    at on each side (`@ interp/C`)."""
    out = []
    wsites, gsites = wsites or {}, gsites or {}
    wslots, gslots = want.get("__slots", {}), got.get("__slots", {})
    for name in sorted((set(want) | set(got)) - {"__slots"}, key=lambda n: [int(x) if x.isdigit() else x for x in re.split(r"(\d+)", n)]):
        if name not in got:
            out.append(f"store {name}: live in the interpreter, not in the C")
            continue
        if name not in want:
            out.append(f"store {name}: live in the C, not in the interpreter")
            continue
        (wh, wr), (gh, gr) = want[name], got[name]
        tag = f"{name} @{wsites.get(name, '?')}/{gsites.get(name, '?')}"
        if wh[0] != gh[0]:
            out.append(f"store {tag}: root type {wh[0]} in the interpreter, {gh[0]} in the C")
        for rec in sorted(set(wr) | set(gr)):
            if rec not in gr:
                out.append(f"store {tag} record {rec}: claimed in the interpreter only")
            elif rec not in wr:
                out.append(f"store {tag} record {rec}: claimed in the C only")
            elif wr[rec][0] != gr[rec][0]:
                out.append(f"store {tag} record {rec}: {wr[rec][0]} words in the interpreter, {gr[rec][0]} in the C")
            elif wr[rec][1] != gr[rec][1]:
                masked = (noise or {}).get((name, rec), set())
                for lo, hi, x, y in byte_runs(wr[rec][1], gr[rec][1]):
                    if same_store_refs(wr[rec][1], gr[rec][1], lo, hi, wslots, gslots):
                        continue
                    if is_unwritten(x) != is_unwritten(y) and (is_zero(x) or is_zero(y)):
                        span = f"byte {lo}" if hi == lo + 1 else f"bytes {lo}..{hi - 1}"
                        side = "the interpreter" if is_zero(x) else "the C"
                        out.append(f"store {tag} record {rec}: {span} zeroed by {side}, unwritten by the other "
                                   f"(padding, if no field covers it)")
                        continue
                    if all(k in masked for k in range(lo, hi)):
                        out.append(f"store {tag} record {rec}: {hi - lo} byte(s) at {lo} vary from run "
                                   f"to run (masked: a clock, an address — nothing a seed fixes)")
                        continue
                    span = f"byte {lo}" if hi == lo + 1 else f"bytes {lo}..{hi - 1}"
                    out.append(f"store {tag} record {rec}: {span} {x} in the interpreter, {y} in the C")
    return out


def compare(prog, env, plant=False):
    """`(None, why)` when not compared, else `(lines, stores, records)`.  Both sides hash with
    one fixed seed (`LOFT_HASH_SEED`), so a hash table's seed and its entries' places are the
    same: a table is pseudo-random, and seeded alike it is comparable byte for byte."""
    env = {"LOFT_HASH_SEED": "0x9E3779B97F4A7C15", **env}
    with tempfile.TemporaryDirectory() as d:
        try:
            cres, why = run_c(prog, d, env, plant)
            if cres is None:
                return None, why
            got, gsites = cres
            want, wsites = run_interp(prog, d, env)
            # The noise control: a second interpreter run.  A byte that differs between two
            # runs of one side varies by itself (a seed, a clock, an address) and is masked.
            d2 = os.path.join(d, "again")
            os.makedirs(d2)
            again, _ = run_interp(prog, d2, env)
        except Exception as e:  # noqa: BLE001 — a probe that cannot run says why and moves on
            return None, f"error {type(e).__name__}: {e}"
    records = sum(len(v[1]) for k, v in want.items() if k != "__slots")
    return differences(want, got, wsites, gsites, noise_of(want, again)), len(want) - 1, records


def noise_of(a, b):
    """`{(store, record): {byte}}` — the bytes two runs of one side disagree on."""
    out = {}
    for name in (set(a) & set(b)) - {"__slots"}:
        for rec in set(a[name][1]) & set(b[name][1]):
            x, y = a[name][1][rec][1], b[name][1][rec][1]
            for lo, hi, _, _ in byte_runs(x, y):
                out.setdefault((name, rec), set()).update(range(lo, hi))
    return out


def attribute(prog):
    """The first C-only rewrite whose switch makes the stores agree, or None."""
    for rule, switch in cad.C_ONLY_REWRITES:
        r = compare(prog, {**os.environ, switch: "1"})
        if r[0] is not None and not r[0]:
            return rule, switch
    every = {**os.environ, **{s: "1" for _, s in cad.C_ONLY_REWRITES}}
    r = compare(prog, every)
    if r[0] is not None and not r[0]:
        return " + ".join(x for x, _ in cad.C_ONLY_REWRITES), "every C-only rewrite off"
    return None


def main():
    args = sys.argv[1:]
    attr = "--attribute" in args
    plant = "--plant" in args
    files = [a for a in args if not a.startswith("--")]
    if not files:
        print(__doc__)
        return 2
    failed = 0
    for prog in files:
        if "fn main(" not in open(prog, encoding="utf-8", errors="replace").read():
            print(f"{prog}: skipped (no main)")
            continue
        r = compare(prog, dict(os.environ))
        if r[0] is None:
            print(f"{prog}: skipped ({r[1]})")
            continue
        lines, stores, records = r
        if plant:
            # The control: the planted byte must add a difference the honest run does not have.
            p = compare(prog, dict(os.environ), True)
            added = [x for x in (p[0] or []) if x not in lines] if p[0] is not None else []
            if added:
                print(f"{prog}: the planted byte is reported — {added[0]}")
            else:
                print(f"{prog}: the planted byte was NOT reported: the comparison is blind")
                failed += 1
            continue
        padding = [x for x in lines if x.endswith("(padding, if no field covers it)")]
        lines = [x for x in lines if x not in padding]
        if not lines:
            extra = f", apart from {len(padding)} run(s) of padding" if padding else ""
            print(f"{prog}: same ({stores} stores, {records} records{extra})")
            for line in padding[:5]:
                print("    " + line)
            continue
        print(f"{prog}: {len(lines)} difference(s)")
        for line in lines[:20]:
            print("    " + line)
        if len(lines) > 20:
            print(f"    … and {len(lines) - 20} more")
        if attr and not plant:
            a = attribute(prog)
            print(f"    the rewrite's: {a[0]} (agrees under {a[1]})" if a else "    no C-only rewrite accounts for it")
            failed += a is None
        elif not plant:
            failed += 1
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
