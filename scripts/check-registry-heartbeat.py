#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Is the registry's signed index and advisory feed recent enough?

    python3 scripts/check-registry-heartbeat.py [--base URL] [--now YYYY-MM-DDThh:mm:ssZ]

The registry re-signs `index.json` and `advisories.json` with their `updated` set to the
moment of signing (`scripts/registry-heartbeat.sh`, run before a PR is opened).  A signed
`updated` therefore says the document was current no earlier than that, and a client can
refuse one too old to be the current document — an old feed served again cannot hide an
advisory for longer than that.  This check keeps the heartbeat going: the `registry-heartbeat`
job of ci.yml runs it on every PR and every day.

Exit codes, one per answer:

    0  both signed less than 6 days ago            — silent
    2  the older was signed 6 days ago             — the last day: re-sign now (loud, not red)
    1  the older was signed 7 or more days ago     — blocked: re-sign, then run the check again
    3  a document could not be read                — blocked: nothing was measured

It reads the `updated` stamps only; the signatures are the client's check, and the heartbeat
verifies its own against the trusted keys before it pushes.
"""
import argparse
import datetime
import json
import sys
import urllib.request

DOCS = ("index.json", "advisories.json")
BASE = "https://raw.githubusercontent.com/loft-lang/registry/main"
FMT = "%Y-%m-%dT%H:%M:%SZ"
WARN_DAYS = 6
BLOCK_DAYS = 7


def stamp(text):
    return datetime.datetime.strptime(text, FMT).replace(tzinfo=datetime.timezone.utc)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", default=BASE, help="where the two documents are read from")
    ap.add_argument("--now", help="the moment to measure against (default: now)")
    a = ap.parse_args()
    now = stamp(a.now) if a.now else datetime.datetime.now(datetime.timezone.utc)

    signed = {}
    for doc in DOCS:
        url = f"{a.base}/{doc}"
        try:
            with urllib.request.urlopen(url, timeout=60) as r:
                signed[doc] = stamp(json.loads(r.read())["updated"])
        except Exception as e:  # noqa: BLE001 — any failure is "not measured", said with its cause
            print(f"registry heartbeat: cannot read the `updated` of {url}: {e}")
            return 3

    doc, oldest = min(signed.items(), key=lambda kv: kv[1])
    age = now - oldest
    days = age.days
    blocks = oldest + datetime.timedelta(days=BLOCK_DAYS)
    when = f"{doc} was signed {oldest:%Y-%m-%d %H:%M} UTC, {days} days ago"
    cure = "run scripts/registry-heartbeat.sh on the machine that holds the registry key"
    if days >= BLOCK_DAYS:
        print(f"registry heartbeat: BLOCKED — {when}; a PR cannot merge until it is re-signed: {cure}")
        return 1
    if days >= WARN_DAYS:
        print(f"registry heartbeat: LAST DAY — {when}; every PR is blocked from "
              f"{blocks:%Y-%m-%d %H:%M} UTC unless it is re-signed before then: {cure}")
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
