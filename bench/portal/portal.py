#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""portal.py — ONE page that says where loft-native stands against Rust, by CLASS.

    python3 bench/portal/portal.py measure [stats.py options]      # e.g. --only 13,15
    python3 bench/portal/portal.py render

`measure` runs `bench/stats.py` over the suite and over every library bench `libs.tsv`
names that `checkout_libs.sh` has checked out, and MERGES the run into
`bench/portal/results/<host>.tsv` — one file per machine, because a ratio is between two
lanes on ONE machine and the machine is part of every row.  A partial run (`--only`,
`--no-suite`) replaces the rows it re-measured and keeps the rest; `--no-packages` skips the
library benches; an explicit `--package DIR=NAME` replaces the default list.  `render` joins every
saved run with the registries beside this script and writes `doc/claude/PERF_PORTAL.md`:

  classes.tsv   what each mechanism class is
  routines.tsv  every measured routine -> its class and its population
  census.tsv    library routines worth a row and not measured yet

The page leads with the CLASS table — the question it exists to answer is which KINDS of
routine are still too slow, so that a pass starts from a class and not from one program —
then every routine under its class, then coverage: which libraries are measured, and which
surveyed routines are waiting.  @PLN158; the rules are formal/performance.md.

The page is GENERATED: edit a registry or re-measure, never the page.
"""
import os
import statistics
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
BENCH = os.path.dirname(HERE)
ROOT = os.path.dirname(BENCH)
RESULTS = os.path.join(HERE, "results")
PAGE = os.path.join(ROOT, "doc", "claude", "PERF_PORTAL.md")
BAR, CEILING = 2.0, 3.0


def read_tsv(path, columns):
    rows = []
    with open(path) as f:
        for line in f:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            cells = line.split("\t")
            if len(cells) < columns:
                sys.exit(f"{path}: a row has {len(cells)} columns, want {columns}: {line[:80]}")
            rows.append(cells)
    return rows


def read_run(path):
    meta, header, rows = {}, None, []
    with open(path) as f:
        for line in f:
            line = line.rstrip("\n")
            if line.startswith("# "):
                k, _, v = line[2:].partition("=")
                meta[k] = v
            elif line and header is None:
                header = line.split("\t")
            elif line:
                rows.append(dict(zip(header, line.split("\t"))))
    return meta, rows


def library_packages():
    """`--package` arguments for every library bench libs.tsv names and a checkout holds
    (`checkout_libs.sh` makes the checkouts; $LOFT_PERF_LIBS says where)."""
    dest = os.environ.get("LOFT_PERF_LIBS", os.path.join(os.path.dirname(ROOT), "loft-bench-libs"))
    args = []
    for repo, _branch, packages in read_tsv(os.path.join(HERE, "libs.tsv"), 3):
        for spec in packages.split(","):
            name, _, sub = spec.partition("=")
            pkg = os.path.join(dest, repo, sub)
            if sub and os.path.exists(os.path.join(pkg, "bench", "bench.loft")):
                args += ["--package", f"{pkg}={name}"]
    return args


def measure(argv):
    """Run stats.py and MERGE what it measured into this machine's results file: a partial
    run (`--only`, `--no-suite`) replaces the rows it re-measured and leaves the others, and
    every row carries the commit and date it was measured at."""
    import platform
    import tempfile
    os.makedirs(RESULTS, exist_ok=True)
    out = os.path.join(RESULTS, f"{platform.node() or 'unknown'}.tsv")
    packages = [] if "--package" in argv or "--no-packages" in argv else library_packages()
    argv = [x for x in argv if x != "--no-packages"]
    with tempfile.NamedTemporaryFile(suffix=".tsv", delete=False) as tmp:
        fresh = tmp.name
    cmd = [sys.executable, os.path.join(BENCH, "stats.py"), "--tsv", fresh, *packages, *argv]
    print(" ".join(cmd), flush=True)
    code = subprocess.call(cmd)
    if code != 0:
        os.remove(fresh)
        sys.exit(code)
    meta, rows = read_run(fresh)
    os.remove(fresh)
    kept = []
    if os.path.exists(out):
        _, old = read_run(out)
        redone = {(r["bench"], r["routine"]) for r in rows}
        kept = [r for r in old if (r["bench"], r["routine"]) not in redone]
    merged = sorted(kept + rows, key=lambda r: (r["bench"], r["routine"]))
    keys = ["bench", "routine"] + sorted({k for r in merged for k in r} - {"bench", "routine"})
    with open(out, "w") as f:
        for k, v in meta.items():
            f.write(f"# {k}={v}\n")
        f.write("\t".join(keys) + "\n")
        for r in merged:
            f.write("\t".join(r.get(k, "") for k in keys) + "\n")
    print(f"saved {os.path.relpath(out, ROOT)}: {len(rows)} row(s) measured, {len(kept)} kept — "
          f"now: python3 bench/portal/portal.py render")


def verdict_mark(ratio):
    if ratio <= BAR:
        return "ok"
    return "over 2x" if ratio <= CEILING else "**over 3x**"


def fmt_ns(ns):
    ns = float(ns)
    if ns >= 1e6:
        return f"{ns / 1e6:,.2f} ms"
    if ns >= 1e3:
        return f"{ns / 1e3:,.1f} µs"
    return f"{ns:,.0f} ns"


def render():
    classes = read_tsv(os.path.join(HERE, "classes.tsv"), 2)
    class_order = [c[0] for c in classes]
    class_what = {c[0]: c[1] for c in classes}
    registry = {(r[0], r[1]): dict(cls=r[2], pop=r[3], what=r[4])
                for r in read_tsv(os.path.join(HERE, "routines.tsv"), 5)}
    census = [dict(lib=r[0], routine=r[1], cls=r[2], kind=r[3], rank=r[4], why=r[5], workload=r[6], twin=r[7])
              for r in read_tsv(os.path.join(HERE, "census.tsv"), 8)]
    for r in registry.values():
        if r["cls"] not in class_what:
            sys.exit(f"routines.tsv names a class classes.tsv lacks: {r['cls']}")
    for c in census:
        if c["cls"] not in class_what:
            sys.exit(f"census.tsv names a class classes.tsv lacks: {c['cls']}")

    runs = []
    if os.path.isdir(RESULTS):
        for name in sorted(os.listdir(RESULTS)):
            if name.endswith(".tsv"):
                runs.append(read_run(os.path.join(RESULTS, name)))
    if not runs:
        sys.exit("no saved run under bench/portal/results — run `portal.py measure` first")

    out = []
    w = out.append
    w("<!--\nCopyright (c) 2026 Jurjen Stellingwerff\nSPDX-License-Identifier: LGPL-3.0-or-later\n-->\n")
    w("# Performance portal — loft native against Rust, by class\n")
    w("> **GENERATED by `bench/portal/portal.py render` — edit a registry or re-measure, never this page.**")
    w("> Measure with `make perf-portal` (`make perf-libs` first, once: it clones the libraries whose own")
    w("> benches join the run).  How a row is measured, and the four rules that keep it")
    w("> like-for-like: [bench/README.md](../../bench/README.md).  The rules a number must meet before it")
    w("> may be compared, and the bar: [formal/performance.md](formal/performance.md) `(Perf-Like)`,")
    w("> `(Perf-Weight)`.  The plan this page is the report of: @PLN158.\n")
    w(f"**The bar:** every routine within **{CEILING:.0f}×** its Rust twin, and — the owner's goal — every")
    w(f"routine within **{BAR:.0f}×**.  A ratio is native ns/op over the twin's, medians of pinned, calibrated,")
    w("interleaved samples; both lanes printed the same result hash or the row would not be here.\n")

    for meta, rows in runs:
        judged = []
        unclassified = []
        for r in rows:
            if not r.get("ratio"):
                continue
            key = (r["bench"], r["routine"])
            reg = registry.get(key)
            if reg is None:
                unclassified.append(key)
                continue
            judged.append(dict(r, **reg, ratio_f=float(r["ratio"])))
        if not judged:
            continue
        host = meta.get("host", "?")
        w(f"## {host} · {meta.get('arch', '?')} · {meta.get('date', '?')}\n")
        w(f"Commit `{meta.get('commit', '?')}`{' (uncommitted changes in the tree)' if meta.get('dirty') == 'yes' else ''}, "
          f"{meta.get('rustc', 'rustc ?')}, reference `rustc {meta.get('ref_flags', '-O')}`, "
          f"{meta.get('samples', '?')} samples of ~{float(meta.get('target_ms', 0)):.0f} ms, "
          f"{'pinned to the fastest core' if meta.get('pinned') == 'yes' else 'NOT pinned'}.\n")

        # ── headline ──
        def tally(rs):
            xs = [r["ratio_f"] for r in rs]
            return (len(xs), statistics.median(xs), sum(x <= BAR for x in xs),
                    sum(BAR < x <= CEILING for x in xs), sum(x > CEILING for x in xs))

        w("### Where we stand\n")
        w("| population | routines | median | within 2× | 2–3× | over 3× |")
        w("|---|---:|---:|---:|---:|---:|")
        pops = [("every measured routine", judged),
                ("shipped routines — stdlib + libraries (the `(Perf-Weight)` population)",
                 [r for r in judged if r["pop"] != "engine"]),
                ("stdlib", [r for r in judged if r["pop"] == "stdlib"]),
                ("libraries", [r for r in judged if r["pop"].startswith("library:")]),
                ("engine programs (informational)", [r for r in judged if r["pop"] == "engine"])]
        for label, rs in pops:
            if rs:
                n, med, ok, mid, over = tally(rs)
                w(f"| {label} | {n} | **{med:.2f}×** | {ok} | {mid} | {over} |")
        w("")

        # ── by class ──
        w("### By class — which KINDS of routine are slow\n")
        w("Sorted by the class median.  A class is the mechanism a routine's cost is made of, so a slow")
        w("class points at one part of the compiler or runtime rather than at one program.\n")
        w("| class | what bounds it | rows | median | best | worst | within 2× | over 3× |")
        w("|---|---|---:|---:|---:|---|---:|---:|")
        by_class = {}
        for r in judged:
            by_class.setdefault(r["cls"], []).append(r)
        ranked = sorted(by_class.items(), key=lambda kv: -statistics.median(x["ratio_f"] for x in kv[1]))
        for cls, rs in ranked:
            xs = sorted(rs, key=lambda r: r["ratio_f"])
            med = statistics.median(r["ratio_f"] for r in rs)
            worst = xs[-1]
            w(f"| **{cls}** | {class_what[cls]} | {len(rs)} | **{med:.2f}×** | {xs[0]['ratio_f']:.2f}× | "
              f"{worst['ratio_f']:.2f}× `{worst['routine']}` | {sum(r['ratio_f'] <= BAR for r in rs)} | "
              f"{sum(r['ratio_f'] > CEILING for r in rs)} |")
        unmeasured = [c for c in class_order if c not in by_class]
        if unmeasured:
            w(f"\nNo measured row yet: {', '.join(f'**{c}**' for c in unmeasured)} — see *Waiting* below.")
        w("")

        # ── every routine ──
        any_older = []
        w("### Every routine, under its class\n")
        for cls, rs in ranked:
            w(f"#### {cls} — {class_what[cls]}\n")
            w("| routine | lane | population | × Rust | range | native | Rust | | what it stands for |")
            w("|---|---|---|---:|---|---:|---:|---|---|")
            for r in sorted(rs, key=lambda r: -r["ratio_f"]):
                flags = f" ({r['flags']})" if r.get("flags") else ""
                older = r.get("commit") and r["commit"] != meta.get("commit")
                if older:
                    flags += f" †{r['commit']}"
                    any_older.append(r)
                w(f"| `{r['routine']}` | {r['bench']} | {r['pop']} | **{r['ratio_f']:.2f}** | "
                  f"{float(r['ratio_lo']):.2f}–{float(r['ratio_hi']):.2f}{flags} | {fmt_ns(r['native_ns'])} | "
                  f"{fmt_ns(r['rust_ns'])} | {verdict_mark(r['ratio_f'])} | {r['what']} |")
            w("")
        if any_older:
            w(f"† measured at an earlier commit than `{meta.get('commit', '?')}` (a partial run re-measures "
              f"only what it names); the commit follows the mark.\n")
        if unclassified:
            w("#### UNCLASSIFIED — a lane prints these and `routines.tsv` lacks them\n")
            for b, n in unclassified:
                w(f"- `{b}` / `{n}`")
            w("")

    # ── coverage ──
    measured_libs = sorted({r["pop"].split(":", 1)[1] for r in registry.values() if r["pop"].startswith("library:")})
    w("## Coverage — what is measured, and what is waiting\n")
    w(f"Libraries with a measured lane: {', '.join(f'**{m}**' for m in measured_libs) or 'none'}.  "
      f"The standard library has {sum(r['pop'] == 'stdlib' for r in registry.values())} rows.\n")
    w("### Waiting — surveyed library routines without a row yet\n")
    w("From a read of each library's source against its published API.  `rank 1` is the survey's first")
    w("pick for its tree.  A routine moves from here into a lane the day its twin is written; the twin")
    w("column says what that is.\n")
    w("| class | waiting | libraries |")
    w("|---|---:|---|")
    for cls in class_order:
        cs = [c for c in census if c["cls"] == cls]
        if cs:
            libs = sorted({c["lib"] for c in cs})
            w(f"| {cls} | {len(cs)} | {', '.join(libs)} |")
    w("")
    for cls in class_order:
        cs = [c for c in census if c["cls"] == cls]
        if not cs:
            continue
        w(f"#### {cls}\n")
        w("| library | routine | | why a program spends time here | workload | the Rust twin |")
        w("|---|---|---|---|---|---|")
        for c in sorted(cs, key=lambda c: (c["rank"], c["lib"], c["routine"])):
            mark = "**1**" if c["rank"] == "1" else "2"
            kind = "" if c["kind"] == "loft" else f" ({c['kind']})"
            w(f"| {c['lib']} | `{c['routine']}`{kind} | {mark} | {c['why']} | {c['workload']} | {c['twin']} |")
        w("")

    os.makedirs(os.path.dirname(PAGE), exist_ok=True)
    with open(PAGE, "w") as f:
        f.write("\n".join(out).rstrip("\n") + "\n")
    print(f"wrote {os.path.relpath(PAGE, ROOT)}")


def main():
    if len(sys.argv) < 2 or sys.argv[1] not in ("measure", "render"):
        sys.exit(__doc__)
    if sys.argv[1] == "measure":
        measure(sys.argv[2:])
    render()


if __name__ == "__main__":
    main()
