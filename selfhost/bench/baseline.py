# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""A survey's committed position: per layer, the count that agrees and every item that does not yet.

@PLN91.  Shared by the surveys of the loft implementation against the Rust one
(`run_image_survey.py`, `bgen_survey.py`), so each gates its position the same way.  The file:

    suite   <sha>                                   the language suite it was written against
    layer   <layer>  agree  <n>  of  <total>        heads each layer's block
    <layer> <verdict> <item>                        an item of that layer that does not agree yet

`write` replaces one layer's block and keeps the others (all of them dropped when the suite
moved: another suite is another corpus).  `check` fails on a DROP — an unlisted item that does
not agree, an item new to the corpus that does not agree, a lower count — and on any FATAL
verdict, listed or not (a wrong answer or wrong bytes); a listed item that agrees now is a
re-pin, which does not fail.  Re-pin only in the commit whose change moves the position, with
the reason in its message.
"""
import os, subprocess

HEAD = """\
# A survey's committed position (selfhost/bench/baseline.py): written by --write-baseline layer by
# layer, checked by --check, where a drop or a fatal verdict fails.  `suite` is the language
# suite commit it was written against (tests/loft-suite.pin); `layer` heads each block; each other
# row is an item of that layer that does not agree yet.  Re-pin only in the commit whose change
# moves it, with the reason.
"""


def read(path):
    """{layer: (agree, total, {item: verdict})}, the suite commit under `suite`; {} when absent."""
    out = {}
    if not os.path.exists(path):
        return out
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            if line.startswith("#") or not line.strip():
                continue
            f = line.rstrip("\n").split("\t")
            if f[0] == "suite":
                out["suite"] = f[1]
            elif f[0] == "layer":
                out[f[1]] = (int(f[3]), int(f[5]), {})
            else:
                out[f[0]][2][f[2]] = f[1]
    return out


def write(path, pin, layer, agree, total, rows):
    """Replace `layer`'s block of `path` with `agree` of `total` and `rows` ({item: verdict})."""
    base = read(path)
    if base.get("suite") not in (None, pin):
        print(f"the suite moved to {pin}: the other layers' blocks are dropped; write them again")
        base = {}
    base[layer] = (agree, total, rows)
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(HEAD)
        fh.write(f"suite\t{pin}\n")
        for name in sorted(k for k in base if k != "suite"):
            a, t, r = base[name]
            fh.write(f"\nlayer\t{name}\tagree\t{a}\tof\t{t}\n")
            for item in sorted(r):
                fh.write(f"{name}\t{r[item]}\t{item}\n")
    print(f"wrote the `{layer}` block of {path}")


def check(layer, base, agree, total, results, fatal=("differ",), good="agree"):
    """The failures of `results` ({item: verdict}) against `layer`'s block, and its re-pins."""
    if layer not in base:
        return [f"the baseline has no `{layer}` block: write it with --write-baseline"], []
    b_agree, _, rows = base[layer]
    fails, repins = [], []
    for item, verdict in sorted(results.items()):
        if verdict in fatal:
            fails.append(f"{verdict}\t{item}")
        elif verdict != good and item not in rows:
            fails.append(f"drop\t{item}\t{verdict}")
        elif verdict == good and item in rows:
            repins.append(f"agrees now\t{item}\twas {rows[item]}")
    repins += [f"no longer in the corpus\t{p}" for p in sorted(set(rows) - set(results))]
    if agree < b_agree:
        fails.append(f"{layer}: {agree} of {total} agree, the baseline {b_agree}")
    return fails, repins


def report(layer, path, fails, repins):
    """Print the re-pins and failures; exit 1 on a failure."""
    for r in repins:
        print(f"re-pin: {r}")
    if fails:
        print(f"the `{layer}` position fell against {path}:")
        for f in fails:
            print(f"  {f}")
        raise SystemExit(1)


def require_pin(root, suite, check_path=None):
    """The pinned suite commit; exits naming both commits when the checkout at `suite` is at
    another, or when the baseline at `check_path` was written against another."""
    with open(os.path.join(root, "tests", "loft-suite.pin"), encoding="utf-8") as fh:
        pin = next(l.strip() for l in fh if l.strip() and not l.startswith("#"))
    p = subprocess.run(["git", "-C", suite, "rev-parse", "HEAD"], capture_output=True, text=True)
    have = p.stdout.strip() if p.returncode == 0 else None
    if have != pin:
        raise SystemExit(f"the language suite at {suite} is at {have or 'no commit'}, the baseline reads "
                         f"the pinned {pin} (tests/loft-suite.pin): check that commit out, or point "
                         f"LOFT_SUITE_DIR at a checkout of it")
    recorded = read(check_path).get("suite") if check_path else None
    if recorded and recorded != pin:
        raise SystemExit(f"{check_path} was written against the suite at {recorded}; this tree pins {pin}: "
                         f"write it again (--write-baseline) in the commit that moves the pin")
    return pin


def name_of(path, root, suite):
    """An item's machine-free name: a suite cell under `loft-suite/`, anything else from `root`."""
    suite_abs = os.path.abspath(suite)
    rel = ("loft-suite/" + os.path.relpath(path, suite_abs) if path.startswith(suite_abs + os.sep)
           else os.path.relpath(path, root))
    return rel.replace(os.sep, "/")
