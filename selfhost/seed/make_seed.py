#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Write the loft runner's seed: the C that builds it with a C compiler alone.

@PLN91, doc/claude/plans/91-self-host/rust-free-build.md, level 1.  The seed is the bytecode
runner's C unit (`loft --native-c --ship --native-emit` over selfhost/bench/run_image_run.loft,
the C runtime inlined), the crt shim it calls (selfhost/crt/shim/crt.c), the compiled
standard-library bodies the shim includes (`loft --emit-c-bodies`), the runtime headers the
shim includes (src/c/*.h) and `build.sh` (selfhost/seed/build.sh).  The paths of this checkout
are removed from the unit and the shim's hashed file name is made plain, so the same sources
give the same seed on any machine; `MANIFEST.sha256` lists every file's hash.

    python3 selfhost/seed/make_seed.py [--out target/seed] [--tar]
    python3 selfhost/seed/make_seed.py --check SEED     # a fresh seed equals SEED, file by file
    python3 selfhost/seed/make_seed.py --build [--smoke N]
        # build the seed under `env -i PATH=/usr/bin:/bin` and compare its runner with the
        # loft-built one on N corpus programs

Writing a seed needs the Rust `loft` (target/release/loft); building one needs only `cc`.
"""
import argparse, hashlib, os, re, shutil, subprocess, sys, tarfile, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
UNIT = "gen/run_image_run.c"
BODIES = "gen/loft_bodies.c"


def run(cmd, cwd=None, env=None, timeout=1800):
    p = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout)
    if p.returncode != 0:
        sys.exit(f"{' '.join(cmd)} failed:\n{p.stderr[-3000:]}")
    return p


def write_seed(out, loft):
    if os.path.isdir(out):
        shutil.rmtree(out)
    for d in ("gen", "src/c", "selfhost/crt/shim"):
        os.makedirs(os.path.join(out, d))
    env = {**os.environ, "LOFT_NO_WARNINGS": "1"}
    unit = os.path.join(out, UNIT)
    run([loft, "--native-c", "--ship", "--native-emit", unit, "--lib", "src", "--lib", "crt/src",
         "bench/run_image_run.loft"], cwd=os.path.join(ROOT, "selfhost"), env=env)
    with tempfile.TemporaryDirectory() as tmp:
        stub = os.path.join(tmp, "stub.loft")
        with open(stub, "w", encoding="utf-8") as fh:
            fh.write("fn main() {\n}\n")
        run([loft, "--emit-c-bodies", os.path.join(out, BODIES), stub], env=env)
    # The checkout's root is dropped from every path the unit names (the frames' source files,
    # the link comments), and the shim's file name loses the hash of the binary that built it.
    text = open(unit, encoding="utf-8", errors="surrogateescape").read()
    text = text.replace(ROOT + os.sep, "")
    text = re.sub(r"libcrt_shim_[0-9a-f]{16}", "libcrt_shim", text)
    with open(unit, "w", encoding="utf-8", errors="surrogateescape") as fh:
        fh.write(text)
    for h in sorted(os.listdir(os.path.join(ROOT, "src", "c"))):
        if h.endswith(".h"):
            shutil.copy(os.path.join(ROOT, "src", "c", h), os.path.join(out, "src", "c", h))
    # The shim includes `<loft_width.h>` and `<loft_bodies.c>` from its include path (gen/).
    shutil.copy(os.path.join(ROOT, "src", "c", "loft_width.h"), os.path.join(out, "gen", "loft_width.h"))
    shutil.copy(os.path.join(ROOT, "selfhost", "crt", "shim", "crt.c"),
                os.path.join(out, "selfhost", "crt", "shim", "crt.c"))
    shutil.copy(os.path.join(ROOT, "selfhost", "seed", "build.sh"), os.path.join(out, "build.sh"))
    os.chmod(os.path.join(out, "build.sh"), 0o755)
    with open(os.path.join(out, "MANIFEST.sha256"), "w", encoding="utf-8") as fh:
        for rel, digest in manifest(out):
            fh.write(f"{digest}  {rel}\n")


def manifest(seed):
    """(path, sha256) of every file of the seed but the manifest, sorted."""
    out = []
    for d, _, fs in os.walk(seed):
        for f in fs:
            rel = os.path.relpath(os.path.join(d, f), seed)
            if rel == "MANIFEST.sha256" or rel == "run_image_run":
                continue
            with open(os.path.join(d, f), "rb") as fh:
                out.append((rel, hashlib.sha256(fh.read()).hexdigest()))
    return sorted(out)


def check(seed, loft):
    """Does a seed written now equal `seed`, file by file?  Answers the differing paths."""
    if seed.endswith((".tar.gz", ".tgz")):
        tmp = tempfile.mkdtemp(prefix="loft-seed-given-")
        with tarfile.open(seed) as tf:
            tf.extractall(tmp, filter="data")
        seed = os.path.join(tmp, "seed")
    with tempfile.TemporaryDirectory() as tmp:
        fresh = os.path.join(tmp, "seed")
        write_seed(fresh, loft)
        a, b = dict(manifest(seed)), dict(manifest(fresh))
        return sorted(p for p in set(a) | set(b) if a.get(p) != b.get(p))


def build(seed, smoke, loft):
    """Build `seed` with a C compiler alone, then compare its runner with the loft-built one on
    `smoke` corpus programs (stdout, stderr and exit code)."""
    env = {"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"}
    for k in ("CC", "CFLAGS"):
        if k in os.environ:
            env[k] = os.environ[k]
    run(["sh", os.path.join(seed, "build.sh")], env=env, timeout=3600)
    seeded = os.path.join(seed, "run_image_run")
    if smoke <= 0:
        return 0, 0
    with tempfile.TemporaryDirectory() as tmp:
        out = os.path.join(tmp, "runner")
        os.makedirs(out)
        run([loft, "--native-c", "--ship", "--out", out, "--lib", "src", "--lib", "crt/src",
             "bench/run_image_run.loft"], cwd=os.path.join(ROOT, "selfhost"),
            env={**os.environ, "LOFT_NO_WARNINGS": "1"})
        built = os.path.join(out, "run_image_run")
        scripts = os.path.join(ROOT, "tests", "scripts")
        skip = re.compile(r"@EXPECT_ERROR|@EXPECT_FAIL|@ARGS|@SCRIPT")
        files = [f for f in sorted(os.listdir(scripts))
                 if f.endswith(".loft") and os.path.isfile(os.path.join(scripts, f))]
        same = tried = 0
        for f in files[:: max(1, len(files) // (smoke * 2))]:
            src = open(os.path.join(scripts, f), encoding="utf-8", errors="replace").read()
            if skip.search(src) or not re.search(r"^fn main\s*\(", src, re.M):
                continue
            img = os.path.join(tmp, f + ".run")
            if subprocess.run([loft, "--emit-run-image", img, f], cwd=scripts, capture_output=True,
                              timeout=300).returncode != 0:
                continue
            got = [subprocess.run([r, img], cwd=scripts, capture_output=True, timeout=300)
                   for r in (seeded, built)]
            tried += 1
            if all((g.stdout, g.stderr, g.returncode) == (got[0].stdout, got[0].stderr, got[0].returncode)
                   for g in got):
                same += 1
            else:
                print(f"differs: tests/scripts/{f}")
            if tried >= smoke:
                break
        return same, tried


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--loft", default=os.path.join(ROOT, "target", "release", "loft"))
    ap.add_argument("--out", default=os.path.join(ROOT, "target", "seed"))
    ap.add_argument("--tar", action="store_true", help="also write <out>.tar.gz")
    ap.add_argument("--check", metavar="SEED", help="a seed directory or .tar.gz to compare a fresh one with")
    ap.add_argument("--build", action="store_true", help="build --out with a C compiler alone")
    ap.add_argument("--smoke", type=int, default=40, help="corpus programs --build compares (0: none)")
    o = ap.parse_args()
    if o.check:
        diff = check(o.check, o.loft)
        if diff:
            print(f"the seed {o.check} is not the one these sources give; differing:")
            for p in diff:
                print(f"  {p}")
            sys.exit(1)
        return
    if o.build:
        same, tried = build(o.out, o.smoke, o.loft)
        if o.smoke > 0 and tried == 0:
            sys.exit("no corpus program was compared: the check proved nothing")
        if same != tried:
            sys.exit(f"the seed-built runner differs from the loft-built one on {tried - same} of {tried} programs")
        print(f"the seed builds with a C compiler alone; its runner equals the loft-built one on {tried} programs")
        return
    write_seed(o.out, o.loft)
    if o.tar:
        with tarfile.open(o.out + ".tar.gz", "w:gz") as tf:
            tf.add(o.out, arcname="seed")


if __name__ == "__main__":
    main()
