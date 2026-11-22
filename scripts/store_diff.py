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

What a byte comparison cannot see — a store freed on one side and live on the other — is a
store missing from one dump, reported as such.  A program the C writer refuses, or that has no
`main`, is skipped and says why."""
import os, re, subprocess, sys, tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import c_alloc_diff as cad  # noqa: E402

LOFT = cad.LOFT


def read_dump(path, names):
    """`{history name: (header, {record: (words, hex)})}` for every store the trace names."""
    out, cur = {}, None
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
        elif f[0] == "R" and cur is not None:
            cur[1][int(f[1])] = (int(f[2]), f[3] if len(f) > 3 else "")
    return out


def run_interp(prog, d, env):
    t, dump = os.path.join(d, "i.trace"), os.path.join(d, "i.dump")
    subprocess.run([LOFT, "--interpret", prog],
                   env={**env, "LOFT_ALLOC_TRACE": t, "LOFT_STORE_DUMP": dump, "LOFT_POISON_CLAIM": "1"},
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=300)
    names = cad.program_names(open(t, encoding="utf-8")) if os.path.exists(t) else {}
    return read_dump(dump, names)


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
    names = cad.program_names(open(t, encoding="utf-8")) if os.path.exists(t) else {}
    return read_dump(dump, names), None


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


def differences(want, got):
    """Every difference, as lines: a store one side lacks, a record one side lacks, a record of
    another size, and each differing record's first differing byte."""
    out = []
    for name in sorted(set(want) | set(got), key=lambda n: [int(x) if x.isdigit() else x for x in re.split(r"(\d+)", n)]):
        if name not in got:
            out.append(f"store {name}: live in the interpreter, not in the C")
            continue
        if name not in want:
            out.append(f"store {name}: live in the C, not in the interpreter")
            continue
        (wh, wr), (gh, gr) = want[name], got[name]
        if wh[0] != gh[0]:
            out.append(f"store {name}: root type {wh[0]} in the interpreter, {gh[0]} in the C")
        for rec in sorted(set(wr) | set(gr)):
            if rec not in gr:
                out.append(f"store {name} record {rec}: claimed in the interpreter only")
            elif rec not in wr:
                out.append(f"store {name} record {rec}: claimed in the C only")
            elif wr[rec][0] != gr[rec][0]:
                out.append(f"store {name} record {rec}: {wr[rec][0]} words in the interpreter, {gr[rec][0]} in the C")
            elif wr[rec][1] != gr[rec][1]:
                for lo, hi, x, y in byte_runs(wr[rec][1], gr[rec][1]):
                    span = f"byte {lo}" if hi == lo + 1 else f"bytes {lo}..{hi - 1}"
                    out.append(f"store {name} record {rec}: {span} {x} in the interpreter, {y} in the C")
    return out


def compare(prog, env, plant=False):
    """`(None, why)` when not compared, else `(lines, stores, records)`."""
    with tempfile.TemporaryDirectory() as d:
        try:
            got, why = run_c(prog, d, env, plant)
            if got is None:
                return None, why
            want = run_interp(prog, d, env)
        except Exception as e:  # noqa: BLE001 — a probe that cannot run says why and moves on
            return None, f"error {type(e).__name__}: {e}"
    records = sum(len(r) for _, r in want.values())
    return differences(want, got), len(want), records


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
        if not lines:
            print(f"{prog}: same ({stores} stores, {records} records)")
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
