#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Derive which stdlib builtins exist on which target — loft#680.

Nothing used to state this. A consumer learned that `store_load_key` had no browser
implementation by writing the program, building it for `--html`, and reading a rustc error
against generated Rust — *after* the design that assumed the builtin. This produces the
fact up front, as `index/target_surface.json`.

It is DERIVED, never hand-written, because a hand-kept list is the failure mode one level
up: the same report also found `default/02_files.loft` claiming a stricter restriction than
the code enforced. The derivation is exact rather than heuristic — no `cfg` expression is
ever interpreted:

  1. read each builtin's `#rust` body out of `default/*.loft` and note the runtime methods
     it calls (`stores.foo(...)` / `state.foo(...)`);
  2. emit a probe crate that merely REFERENCES each method (`let _ = Stores::foo;`) — a
     reference needs no arguments, so no call has to be synthesised;
  3. compile it once per target and let RUSTC say which are absent. Its answer is the
     truth by construction: it is the same compiler, the same `cfg`s and the same rlib the
     real build uses.

A builtin is available on a target iff every method it calls exists there.

  scripts/gen_target_surface.py            # write index/target_surface.json
  scripts/gen_target_surface.py --check    # exit 1 if the committed file is stale
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "index" / "target_surface.json"

# Targets we can answer for, and how to build a probe against each. The browser rlib is the
# `--html` shape (`--no-default-features --features random`), which is the configuration
# whose gaps a consumer actually hits.
TARGETS = {
    "wasm-browser": {
        "triple": "wasm32-unknown-unknown",
        "rlib": "target/wasm32-unknown-unknown/release",
        "describe": "the browser target (`loft --html`)",
    },
}

# A `#rust` body reaches the runtime through one of these receivers.
CALL_RE = re.compile(r"\b(?:stores|s\.database|state)\.([a-z_][a-z_0-9]*)\s*\(")
DECL_RE = re.compile(r"^\s*(?:pub\s+)?fn\s+([A-Za-z_][A-Za-z_0-9]*)\s*\(")
RUST_RE = re.compile(r'^#rust\s*"(.*)"\s*$', re.S)


def extract() -> dict[str, list[str]]:
    """Map each builtin to the runtime methods its `#rust` body calls."""
    out: dict[str, list[str]] = {}
    for path in sorted((ROOT / "default").glob("*.loft")):
        # `.loft/` is a cache DIRECTORY and this glob matches it: `pathlib.glob` has no
        # leading-dot exclusion, so `default/.loft` (gitignored, created by any run in that
        # directory) arrives here and `read_text` raises IsADirectoryError.  The same guard
        # with the same reason already sits in `scripts/falsify-review.py` — one question,
        # two decoders, and only one of them had been taught.
        if not path.is_file():
            continue
        last_fn: str | None = None
        for line in path.read_text().splitlines():
            decl = DECL_RE.match(line)
            if decl:
                last_fn = decl.group(1)
                continue
            if line.startswith("#rust") and last_fn:
                body = RUST_RE.match(line)
                if body:
                    methods = sorted(set(CALL_RE.findall(body.group(1))))
                    if methods:
                        out.setdefault(last_fn, [])
                        for m in methods:
                            if m not in out[last_fn]:
                                out[last_fn].append(m)
                last_fn = None
    return out


def missing_methods(methods: list[str], target: dict[str, str]) -> set[str]:
    """Methods absent on `target`, per rustc. Empty when the probe compiles clean."""
    rlib_dir = ROOT / target["rlib"]
    rlib = rlib_dir / "libloft.rlib"
    if not rlib.exists():
        sys.exit(
            f"missing {rlib}\n"
            f"  build it first:  cargo build --release --target {target['triple']} "
            f"--lib --no-default-features --features random"
        )
    src = ["#![allow(unused)]", "use loft::database::Stores;"]
    for i, m in enumerate(methods):
        src.append(f"fn p{i}() {{ let _ = Stores::{m}; }}")
    with tempfile.TemporaryDirectory() as td:
        probe = Path(td) / "probe.rs"
        probe.write_text("\n".join(src) + "\n")
        proc = subprocess.run(
            [
                "rustc", "--edition=2024",
                "--target", target["triple"],
                "--crate-type", "rlib",
                "--extern", f"loft={rlib}",
                "-L", f"dependency={rlib_dir / 'deps'}",
                # The HOST deps too, and not as belt-and-braces: `loft` depends on
                # `wasm_bindgen_macro`, which is a PROC-MACRO — it is compiled for the host
                # and can never appear in a wasm deps directory.  Without this path rustc
                # stops at `E0463: can't find crate for wasm_bindgen_macro` before it type-
                # checks a single probe fn, so NO per-method verdict is emitted and `absent`
                # comes back empty — which is exactly what "nothing is unavailable" looks
                # like.  The self-test below is what catches that, and it caught it here.
                "-L", f"dependency={ROOT / 'target/release/deps'}",
                "--error-format=json",
                "-o", str(Path(td) / "out.rlib"),
                str(probe),
            ],
            capture_output=True, text=True, check=False,
        )
    absent: set[str] = set()
    for line in proc.stderr.splitlines():
        try:
            msg = json.loads(line).get("message", "")
        except json.JSONDecodeError:
            continue
        # "no associated function or constant named `foo` found for struct `Stores`"
        if "no associated function" in msg or "no function or associated item" in msg:
            parts = msg.split("`")
            if len(parts) > 1:
                absent.add(parts[1])
    return absent


def build() -> dict:
    builtins = extract()
    every_method = sorted({m for ms in builtins.values() for m in ms})
    surface: dict = {
        "_comment": (
            "GENERATED by scripts/gen_target_surface.py — do not edit. Derived by asking "
            "rustc which runtime methods exist per target, so it cannot drift from the "
            "cfgs. Regenerate with `make surface-gen`. loft#680."
        ),
        "targets": {},
    }
    for name, target in TARGETS.items():
        absent = missing_methods(every_method, target)
        unavailable = sorted(
            fn for fn, ms in builtins.items() if any(m in absent for m in ms)
        )
        surface["targets"][name] = {
            "describe": target["describe"],
            "triple": target["triple"],
            "unavailable_builtins": unavailable,
            "unavailable_methods": sorted(absent),
        }
    surface["builtin_count"] = len(builtins)
    return surface


def self_test() -> None:
    """Prove the probe can REPORT an absence before any empty result is believed.

    The answer this tool gives is currently "nothing is unavailable", and an empty list is
    exactly what a silently broken probe produces too — a typo in the error match, a rustc
    message reworded, a probe that never compiled. So ask it about a method that certainly
    does not exist and require it to say so.
    """
    target = TARGETS["wasm-browser"]
    sentinel = "loft680_method_that_cannot_exist"
    absent = missing_methods([sentinel], target)
    if sentinel not in absent:
        sys.exit(
            "SELF-TEST FAILED: the probe did not report a method that cannot exist, so an "
            "empty 'unavailable' list proves nothing. Fix the probe before trusting it."
        )
    # …and that a method which DOES exist is not reported (no false positives).
    if missing_methods(["load_bytes"], target):
        sys.exit(
            "SELF-TEST FAILED: the probe reported a method that exists, so it would "
            "condemn working builtins."
        )


def main() -> int:
    check = "--check" in sys.argv
    self_test()
    fresh = build()
    text = json.dumps(fresh, indent=2, sort_keys=True) + "\n"
    if check:
        if not OUT.exists():
            print(f"ERROR: {OUT} is missing. Run `make surface-gen` and commit it.")
            return 1
        if OUT.read_text() != text:
            print(
                f"ERROR: {OUT.relative_to(ROOT)} is stale — the per-target builtin surface "
                "changed.\n  Run `make surface-gen` and commit the result."
            )
            return 1
        for name, t in fresh["targets"].items():
            print(f"{name}: {len(t['unavailable_builtins'])} builtin(s) unavailable")
        print("target surface in sync.")
        return 0
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(text)
    for name, t in fresh["targets"].items():
        print(f"{name}: {len(t['unavailable_builtins'])} of "
              f"{fresh['builtin_count']} builtins unavailable")
        for fn in t["unavailable_builtins"]:
            print(f"  {fn}")
    print(f"wrote {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
