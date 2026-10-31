#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The strict comparison of the two interpreters (@PLN91, doc/claude/plans/91-self-host/1b-compare.md).

    selfhost/compare/compare.py FILE[::test_name] … [--layers out,alloc,…] [--keep]

For each entry — a program's `main`, or one test function — every output both implementations
produce is compared, one LAYER per line: `EQUAL`, `DIFFERENT` (the first differing row, with
context from both sides), `MISSING <side>` (that side does not produce the layer yet — a
failure, not a skip) or `REFUSED` (the loft interpreter refused the program).  The exit code is
0 only when every selected layer of every entry is `EQUAL`.  The normalisations are the ones
1b-compare.md lists, and nothing else.

The Rust side is the `op-census` build (`LOFT_BIN`, default target/op-census/release/loft), which
writes the allocator trace; the loft side is `selfhost/cover/run_program.loft`, compiled through
the C backend into target/selfhost-compare/ and rebuilt when a selfhost source is newer.  Not a
test: it is run by hand, on any program, test script or suite cell."""
import argparse, os, re, subprocess, sys, tempfile, shutil

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
from c_alloc_diff import program_events  # noqa: E402  — one normalisation for both comparators

LOFT = os.environ.get("LOFT_BIN", os.path.join(ROOT, "target", "op-census", "release", "loft"))
LAYERS = ["out", "alloc", "leak", "ir", "types", "schema", "ctable", "ownership", "ownership-overlays",
          "binds", "slots", "lifetimes", "bytecode"]
# The layers the loft side does not produce yet, and the step that builds each (1b-compare.md).
NOT_YET = {
    "ownership-overlays": "after the oracle: per_iteration_frees, text_return_orphan_risk, free_before_dependent_read, return_source_freed, ref_param_publish_freed, heap_return_delivery",
    "lifetimes": "C-3, the compute_intervals port",
    "bytecode": "C-3, with 1b-frame.md F-2",
}
LIBS = ["--lib", os.path.join(ROOT, "tests", "lib")]
# The layers that run the program; the others are compile-time reports, compared on any program
# the front end accepts, whether or not the loft interpreter runs it yet.
RUN_LAYERS = ("out", "alloc", "leak", "binds")


def run(cmd, env=None, timeout=300):
    try:
        p = subprocess.run(cmd, capture_output=True, env={**os.environ, **(env or {})}, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return -9, b"", b"timeout"


def build(program, name):
    """A selfhost program compiled through the C backend into target/selfhost-compare/<name>;
    rebuilt when a selfhost source is newer."""
    out = os.path.join(ROOT, "target", "selfhost-compare", name)
    exe = os.path.join(out, os.path.splitext(os.path.basename(program))[0])
    newest = 0.0
    for d, _, fs in os.walk(os.path.join(ROOT, "selfhost")):
        if "native-auto" in d or os.sep + ".loft" in d:
            continue
        for f in fs:
            if f.endswith((".loft", ".c", ".toml")):
                newest = max(newest, os.path.getmtime(os.path.join(d, f)))
    if not os.path.exists(exe) or os.path.getmtime(exe) < newest:
        os.makedirs(out, exist_ok=True)
        code, _, err = run([LOFT, "--native-c", "--ship", "--out", out,
                            os.path.join(ROOT, "selfhost", program)], timeout=900)
        if not os.path.exists(exe) or os.path.getmtime(exe) < newest:
            sys.exit(f"{program} did not build:\n" + err.decode(errors="replace")[-2000:])
    return exe


def text(b):
    return b.decode("utf-8", errors="replace")


def differ(name, rust, loft):
    """`EQUAL`, or `DIFFERENT` at the first differing row with three rows of context each side."""
    if rust == loft:
        return "EQUAL", []
    i = next((k for k, (a, b) in enumerate(zip(rust, loft)) if a != b), min(len(rust), len(loft)))
    rows = [f"{name}: first difference at row {i + 1} of {len(rust)} (rust) / {len(loft)} (loft)"]
    for k in range(max(0, i - 3), i + 4):
        a = rust[k] if k < len(rust) else "-"
        b = loft[k] if k < len(loft) else "-"
        rows.append(f"  {'>' if k == i else ' '} rust {a!r}")
        rows.append(f"  {'>' if k == i else ' '} loft {b!r}")
    return "DIFFERENT", rows


HARNESS = re.compile(rb"^  (ok|FAIL) ", re.M)


def out_layer(entry_test, diag, rust, loft):
    """stdout, the exit code and stderr past the front end's diagnostics; for a test entry the
    harness block cut from the Rust stdout, its verdict compared with the runner's."""
    rc, rout, rerr = rust
    lc, lout, lerr = loft
    if not rerr.startswith(diag):
        return "DIFFERENT", ["out: the Rust run's stderr does not start with the front end's diagnostics"]
    rerr = rerr[len(diag):]
    if entry_test:
        m = HARNESS.search(rout)
        if not m:
            return "DIFFERENT", ["out: the Rust test run printed no harness line"]
        verdict = m.group(1).decode()
        rout = rout[:m.start()]
        rows = [("stdout", rout.splitlines(), lout.splitlines()),
                ("verdict", [verdict], ["ok" if lc == 0 else "FAIL"])]
        if verdict == "FAIL":
            rows.append(("message", [text(rout[m.start():])], [text(lerr)]))
    else:
        rows = [("stdout", rout.splitlines(), lout.splitlines()),
                ("exit", [str(rc)], [str(lc)]),
                ("stderr", rerr.splitlines(), lerr.splitlines())]
    for name, a, b in rows:
        v, detail = differ(f"out {name}", a, b)
        if v != "EQUAL":
            return v, detail
    return "EQUAL", []


def trimmed(lines):
    out = list(lines)
    while out and out[-1] == "":
        out.pop()
    return out


def leaked(events):
    made, freed = [], set()
    for e in events:
        f = e.split()
        if f[0] == "N":
            made.append(f[1])
        elif f[0] == "F":
            freed.add(f[1])
    return [s for s in made if s not in freed]


def introspect_sections(file):
    """`loft introspect --show-bytecode --show-types`: each function's IR (its `fn` header to the
    line before its `byte-code for` line) and the types table, as lists of lines."""
    rc, out, err = run([LOFT, "introspect", "--show-bytecode", "--show-types"] + LIBS + [file])
    if rc != 0:
        return None, text(err).splitlines()[-3:]
    ir, types, section, take = [], [], None, False
    for ln in text(out).split("\n"):
        if ln.startswith("=== "):
            section, take = ln, False
            continue
        if section == "=== bytecode ===":
            if ln.startswith("byte-code for "):
                take = False
            elif ln.startswith("fn ") and not take:
                take = True
            if take:
                ir.append(ln)
        elif section == "=== types ===":
            types.append(ln)
    # The types section opens with the blank line `introspect` writes before its marker's
    # successor; the report starts at its first function.
    while types and types[0] == "":
        types.pop(0)
    return {"ir": ir, "types": types}, None


def slots_layer(file, img, reports):
    """`introspect --show-slots`'s tables against the frame the loft side reads off the image, the
    one normalisation the live column: the lifetimes layer compares it (compute_intervals)."""
    rc, out, err = run([LOFT, "introspect", "--show-slots"] + LIBS + [file])
    if rc != 0:
        return "MISSING rust", text(err).splitlines()[-3:]
    rows, inside = [], False
    for ln in text(out).split("\n"):
        if ln.startswith("=== "):
            inside = ln == "=== slots ==="
            continue
        if inside:
            ln = re.sub(r"\s+live$", "", ln)
            ln = re.sub(r"^(  \d.*?)\s+(\[\d+, \d+\]|-)$", r"\1", ln)
            rows.append(ln.rstrip())
    lc, loft, lerr = run([reports, "slots", img])
    if lc != 0:
        return "MISSING loft", text(lerr).splitlines()[-3:]
    while rows and rows[0] == "":
        rows.pop(0)
    return differ("slots", trimmed(rows), trimmed([x.rstrip() for x in text(loft).split("\n")]))


def ownership_sections(file):
    """`introspect --show-ownership`: its verdict lines (each function's return, each variable)
    and its overlay lines (`!!` and `⚠`), each a list of lines in order."""
    rc, out, err = run([LOFT, "introspect", "--show-ownership"] + LIBS + [file])
    if rc != 0:
        return None, None, text(err).splitlines()[-3:]
    verdicts, overlays, inside = [], [], False
    for ln in text(out).split("\n"):
        if ln.startswith("=== "):
            inside = ln == "=== ownership ==="
            continue
        if not inside:
            continue
        if ln.startswith("!!") or ln.startswith("  \u26a0") or ln.startswith("  delivery: "):
            overlays.append(ln)
        else:
            verdicts.append(ln)
    return verdicts, overlays, None


def ownership_layer(file, img, reports):
    """The oracle's verdicts (`ownership_of`, `return_ownership`) against the ported oracle's."""
    verdicts, _, why = ownership_sections(file)
    if verdicts is None:
        return "MISSING rust", why
    lc, loft, lerr = run([reports, "ownership", img])
    if lc != 0:
        return "MISSING loft", text(lerr).splitlines()[-3:]
    return differ("ownership", trimmed(verdicts), trimmed(text(loft).split("\n")))


def report_layer(layer, file, img, reports):
    """A compile-time report of `introspect` against the same report of the image."""
    sections, why = introspect_sections(file)
    if sections is None:
        return "MISSING rust", why
    lc, loft, lerr = run([reports, layer, img])
    if lc != 0:
        return "MISSING loft", text(lerr).splitlines()[-3:]
    # The one normalisation (1b-compare.md): trailing empty lines, on both sides.
    return differ(layer, trimmed(sections[layer]), trimmed(text(loft).split("\n")))


C_KINDS = ["SCALAR", "TEXT", "STRUCT", "VECTOR", "ENUM", "CHILD", "HASH", "INDEX", "ORDERED",
           "RADIX", "TRIE", "DBREF"]


def c_type_table(c):
    """The C generator's type table in the emitted C, one row per described type, in
    `types::render`'s text — `tests/selfhost.rs`'s `c_type_table`, the same reading."""
    def numbers(t):
        return [x.strip() for x in re.split(r"[{},]", t) if x.strip()]
    fields, variants, keys, rows = {}, {}, {}, []
    for line in c.splitlines():
        m = re.match(r"static const loft_key (\S+)\[\] = (.*)$", line)
        if m:
            n = numbers(m.group(2).rstrip(";"))
            keys[m.group(1)] = [",".join(n[i:i + 3]) for i in range(0, len(n), 3)]
            continue
        m = re.match(r"static const uint16_t (loft_variants\S*)\[\] = (.*)$", line)
        if m:
            variants[m.group(1)] = numbers(m.group(2).rstrip(";"))
            continue
        m = re.match(r"static const loft_fld (\S+)\[\] = (.*)$", line)
        if m:
            n = numbers(m.group(2).rstrip(";"))
            fields[m.group(1)] = [",".join(n[i:i + 5]) for i in range(0, len(n), 5)]
            continue
        m = re.match(r"\s*\[(\d+)\] = \{(.*)$", line)
        if m and "LOFT_T_" in m.group(2):
            parts = numbers(m.group(2).rstrip(","))
            kind = C_KINDS.index(parts[0].replace("LOFT_T_", ""))
            out = f"{m.group(1)} {kind} {parts[1]} {parts[2]} {parts[3]} {parts[6]} {parts[9]}"
            for f in fields.get(parts[4], [])[:int(parts[3]) if parts[3].isdigit() else 0]:
                out += " " + f
            if variants.get(parts[5]):
                out += " variants=" + ",".join(variants[parts[5]])
            if keys.get(parts[7]):
                out += " keys=" + ";".join(keys[parts[7]])
            # A key pointer of 0: the C generator did not describe this type's keys (it writes a
            # type's keys only when the program's code reads them, `keys_of`).
            rows.append((out, parts[7] != "0"))
    return rows


def ctable_layer(file, d, img, reports):
    """The C generator's table against the loft-derived one, on the types the C describes (the
    one normalisation: a type the C does not use has no row there)."""
    c = os.path.join(d, "p.c")
    rc, _, err = run([LOFT, "--emit-c", c] + LIBS + [file])
    if rc != 0:
        return "MISSING rust", ["the C generator refused the program:"] + text(err).splitlines()[-2:]
    table = c_type_table(open(c, encoding="utf-8", errors="replace").read())
    lc, loft, lerr = run([reports, "ctable", img])
    if lc != 0:
        return "MISSING loft", text(lerr).splitlines()[-3:]
    by_tp = {ln.split(" ", 1)[0]: ln for ln in text(loft).splitlines() if ln}
    rust, mine = [], []
    for row, keys_described in table:
        got = by_tp.get(row.split(" ", 1)[0], "-")
        if not keys_described:
            got = re.sub(r" keys=\S*$", "", got)
        rust.append(row)
        mine.append(got)
    return differ("ctable", rust, mine)


def schema_layer(file, img, reports):
    """`LOFT_DUMP_TYPES` under `--check` (nothing run), its closing `ok` line dropped, against the
    image's schema in the same text."""
    rc, rust, rerr = run([LOFT, "--check"] + LIBS + [file], {"LOFT_DUMP_TYPES": "1"})
    if rc != 0:
        return "MISSING rust", text(rerr).splitlines()[-3:]
    rows = rust.splitlines()
    if rows and rows[-1] == b"ok":
        rows = rows[:-1]
    lc, loft, lerr = run([reports, "schema", img])
    if lc != 0:
        return "MISSING loft", text(lerr).splitlines()[-3:]
    return differ("schema", rows, loft.splitlines())


def bind_rows(path):
    """A `LOFT_TRACE_BIND` file as {function: [row, …]}, rows in the order they were written."""
    by_fn = {}
    for line in open(path, encoding="utf-8").read().splitlines():
        by_fn.setdefault(line.split("\t", 1)[0], []).append(line)
    return by_fn


def binds_layer(rbinds, lbinds):
    """The binds layer (1b-compare.md): the functions the loft side lowered, by name, each one's
    rows in order.  The Rust bytecode writer lowers every function, the loft one those it runs."""
    if not os.path.exists(rbinds):
        return "MISSING rust", [f"no bind trace: {LOFT} predates LOFT_TRACE_BIND"]
    if not os.path.exists(lbinds):
        return "MISSING loft", ["the lowering writes no bind table yet (1b-s3.md B-1)"]
    rust, loft = bind_rows(rbinds), bind_rows(lbinds)
    r, l = [], []
    for name in sorted(loft):
        r += rust.get(name, [f"{name}\t(not lowered)"])
        l += loft[name]
    return differ("binds", r, l)


def compare(spec, exe, reports, layers, keep):
    file, _, test = spec.partition("::")
    report = []
    d = tempfile.mkdtemp(prefix="loft-compare-")
    try:
        img = os.path.join(d, "p.store")
        code, _, diag = run([LOFT, "--emit-image", img] + LIBS + [file])
        if code != 0:
            return [("all", "ERROR", ["the front end refused the program:"] + text(diag).splitlines()[-5:])]
        rtrace, ltrace = os.path.join(d, "rust.trace"), os.path.join(d, "loft.trace")
        refused, rust, loft, rev, lev = None, None, None, None, None
        if any(x in RUN_LAYERS for x in layers):
            if test:
                rcmd = [LOFT] + LIBS + ["--tests", f"{file}::{test}"]
            else:
                rcmd = [LOFT, "--interpret"] + LIBS + [file]
            rbinds = os.path.join(d, "rust.binds")
            rust = run(rcmd, {"LOFT_ALLOC_TRACE": rtrace, "LOFT_TRACE_BIND": rbinds})
            lcmd = [exe, "--compare", img] + (["n_" + test] if test else [])
            lbinds = os.path.join(d, "loft.binds")
            loft = run(lcmd, {"LOFT_CRT_ALLOC_TRACE": ltrace, "LOFT_TRACE_BIND": lbinds})
            if loft[0] == 3 and b"refused: " in loft[2]:
                refused = text(loft[2]).split("refused: ", 1)[1].strip().splitlines()[0]
            rev = program_events(open(rtrace, encoding="utf-8")) if os.path.exists(rtrace) else None
            lev = program_events(open(ltrace, encoding="utf-8")) if os.path.exists(ltrace) else None
        for layer in layers:
            if layer in RUN_LAYERS and refused is not None:
                v, detail = "REFUSED", [refused]
            elif layer == "out":
                v, detail = out_layer(bool(test), diag, rust, loft)
            elif layer in ("alloc", "leak") and rev is None:
                v, detail = "MISSING rust", [f"no allocator trace: {LOFT} is not the op-census build"]
            elif layer in ("alloc", "leak") and lev is None:
                v, detail = "MISSING loft", ["the runner wrote no allocator trace"]
            elif layer == "alloc":
                v, detail = differ("alloc", rev, lev)
            elif layer == "leak":
                v, detail = differ("leak", leaked(rev), leaked(lev))
            elif layer == "binds":
                v, detail = binds_layer(rbinds, lbinds)
            elif layer == "schema":
                v, detail = schema_layer(file, img, reports)
            elif layer == "ctable":
                v, detail = ctable_layer(file, d, img, reports)
            elif layer == "ownership":
                v, detail = ownership_layer(file, img, reports)
            elif layer == "slots":
                v, detail = slots_layer(file, img, reports)
            elif layer in ("ir", "types"):
                v, detail = report_layer(layer, file, img, reports)
            else:
                v, detail = "MISSING loft", [NOT_YET[layer]]
            report.append((layer, v, detail))
        return report
    finally:
        if keep:
            print(f"  (kept {d})")
        else:
            shutil.rmtree(d, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("entries", nargs="+", help="FILE or FILE::test_name")
    ap.add_argument("--layers", default=",".join(LAYERS))
    ap.add_argument("--keep", action="store_true", help="keep each entry's scratch directory")
    a = ap.parse_args()
    layers = [x for x in a.layers.split(",") if x]
    unknown = [x for x in layers if x not in LAYERS]
    if unknown:
        sys.exit(f"unknown layers: {', '.join(unknown)} (known: {', '.join(LAYERS)})")
    if not os.path.exists(LOFT):
        sys.exit(f"no Rust side at {LOFT}: cargo build --release --features op-census "
                 "--target-dir target/op-census --bin loft (or set LOFT_BIN)")
    exe = build(os.path.join("cover", "run_program.loft"), "runner")
    reports = build(os.path.join("compare", "reports.loft"), "reports")
    equal = 0
    for spec in a.entries:
        print(f"== {spec}")
        report = compare(spec, exe, reports, layers, a.keep)
        ok = all(v == "EQUAL" for _, v, _ in report)
        equal += ok
        for layer, v, detail in report:
            print(f"   {layer:<10} {v}")
            for row in detail:
                print(f"              {row}")
    print(f"{equal} of {len(a.entries)} entries equal in every layer ({', '.join(layers)})")
    sys.exit(0 if equal == len(a.entries) else 1)


if __name__ == "__main__":
    main()
