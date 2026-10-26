#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""ab.py — two builds of ONE bench program against each other, as statistics.

    python3 bench/portal/ab.py A B [--routine r,…] [--samples 7] [--target-ms 400]
                                   [--cwd DIR] [--subject S] [--no-pin]

A and B are bench binaries (the row protocol, bench/README.md § The row protocol): the C
or Rust a hand-priced edit produced beside the one it was made from
(`bench/portal/hand_price_c.sh`, `bench/portal/hand_price.sh`), or two runtimes' builds of
the same unit.  The method is stats.py's: `--n` calibrated once (on A, so both run the same
work), one discarded warm-up each, then `--samples` rounds in ALTERNATING order (A B, B A, …)
so a drift in the machine's speed lands on both, every run pinned to the fastest core.
Per routine it answers both medians, B/A with its RANGE (B's first quartile over A's third,
and the reverse) and a verdict that reads the range, not the point: `faster` / `slower`
only when the whole range is past ±2 %, `same` otherwise.  Every routine's hash must agree
between A and B — a price whose answer changed is no price — and a disagreement exits 1.

`--subject S` files both medians in the checkout's local results (portal/local.py) as lanes
`ab-a` / `ab-b`, so the price is kept beside the runs it led to.
"""
import argparse
import os
import statistics
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import stats  # noqa: E402  bench/stats.py: the calibration, sampling and spread it uses


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("a")
    ap.add_argument("b")
    ap.add_argument("--routine", default="", help="comma-separated routines to report")
    ap.add_argument("--samples", type=int, default=7)
    ap.add_argument("--target-ms", type=float, default=400.0)
    ap.add_argument("--cwd", default=None, help="the directory the binaries run in")
    ap.add_argument("--subject", default="")
    ap.add_argument("--no-pin", action="store_true")
    a = ap.parse_args()
    keep_names = {r.strip() for r in a.routine.split(",") if r.strip()}
    keep = (lambda n: n in keep_names) if keep_names else None
    pin = stats.pin_prefix(1, not a.no_pin)
    cmds = {"A": [os.path.abspath(a.a)], "B": [os.path.abspath(a.b)]}
    n = stats.calibrate(cmds["A"], pin, a.target_ms * 1000.0, "A", a.cwd, keep,
                        a.target_ms * 1000.0 * 4)
    for lane, cmd in cmds.items():
        stats.rows_of(cmd, n, pin, f"{lane} warm-up", a.cwd)
    samples = {"A": {}, "B": {}}
    hashes = {"A": {}, "B": {}}
    for i in range(a.samples):
        order = ("A", "B") if i % 2 == 0 else ("B", "A")
        for lane in order:
            for name, r in stats.rows_of(cmds[lane], n, pin, lane, a.cwd).items():
                samples[lane].setdefault(name, []).append(r["ns"])
                hashes[lane].setdefault(name, r["hash"])
    names = [m for m in samples["A"] if (keep is None or keep(m)) and m in samples["B"]]
    print(f"# A {a.a}\n# B {a.b}\n# --n {n} · {a.samples} alternating rounds · "
          f"{'pinned' if pin else 'NOT pinned'}")
    print(f"{'routine':22} {'A ns/op':>12} {'±%':>5} {'B ns/op':>12} {'±%':>5} {'B/A':>6} {'range':>13}  verdict")
    bad = False
    rows = []
    for m in names:
        xa, xb = samples["A"][m], samples["B"][m]
        ma, mb = statistics.median(xa), statistics.median(xb)
        qa1, qa3 = stats.quartiles(xa)
        qb1, qb3 = stats.quartiles(xb)
        ratio = mb / ma if ma else 0.0
        lo = qb1 / qa3 if qa3 else 0.0
        hi = qb3 / qa1 if qa1 else 0.0
        verdict = "faster" if hi < 0.98 else "slower" if lo > 1.02 else "same"
        if hashes["A"][m] != hashes["B"][m]:
            verdict = f"HASH DIFFERS ({hashes['A'][m]} vs {hashes['B'][m]})"
            bad = True
        print(f"{m:22} {ma:12,.0f} {stats.spread(xa):5.1f} {mb:12,.0f} {stats.spread(xb):5.1f} "
              f"{ratio:6.3f} {lo:6.3f}–{hi:6.3f}  {verdict}")
        rows.append((m, ma, mb, ratio))
    if a.subject:
        import local  # noqa: E402  portal/local.py
        bench = os.path.basename(os.path.dirname(os.path.abspath(a.a))) or "-"
        for m, ma, mb, ratio in rows:
            for lane, val in (("ab-a", ma), ("ab-b", mb)):
                local.append(bench, m, [{**local.stamp_now(), "host": stats.host_key(),
                                         "subject": a.subject, "lane": lane,
                                         "value": f"{val:.0f}", "unit": "ns/op", "spread": "",
                                         "ratio": f"{ratio:.3f}" if lane == "ab-b" else "",
                                         "source": "ab.py", "note": f"{a.a} vs {a.b}"}])
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
