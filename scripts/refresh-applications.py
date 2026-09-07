#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN112 — build doc/claude/applications-snapshot.json: the major APPLICATIONS built with loft
# for the "state of the loft distribution" doc. NOT part of the distribution — reference EXAMPLES
# of how to build such apps. No hardcoded list: each app is self-described or issue-sourced.
#
# THREE SOURCES, by ownership (a fact lives where it is maintained):
#   * FIRST-PARTY standalone apps  — self-described in their OWN repo: a `loft-showcase` GitHub
#     topic (discovery) + a `.loft-showcase.toml` (`demonstrates`, optional `demo`/`name`/`url`).
#     name/summary come live from the repo's own description. Edited in the app; zero drift.
#   * loft's OWN in-repo demos     — this repo's `.loft-showcase.toml` (Crystal/Brick/gallery).
#   * COMMUNITY apps               — the `application_showcase` issue: a maintainer promotes an
#     accepted `showcase:pending` submission to `showcase`; its form body carries the metadata.
#
# `origin` = first-party (self-described repos + loft's demos) / community (issues).
#
# Usage:  scripts/refresh-applications.py
from __future__ import annotations

import base64
import json
import re
import subprocess
import sys
try:
    import tomllib
except ModuleNotFoundError:  # python < 3.11 (macOS ships 3.9)
    try:
        import tomli as tomllib
    except ModuleNotFoundError:
        import sys
        sys.exit("needs python >= 3.11 (tomllib) or `pip3 install tomli`")
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "doc" / "claude" / "applications-snapshot.json"
LOCAL_DESCRIPTOR = REPO / ".loft-showcase.toml"
FIRST_PARTY_OWNERS = ["jjstwerff"]  # owners whose `loft-showcase`-topic repos are first-party apps
TOPIC = "loft-showcase"
ISSUE_REPO = "loft-lang/loft"
LIST_LABEL = "showcase"  # listed community apps (`showcase:pending` = intake, not listed)


def gh(args: list[str]) -> str:
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else ""


def entry(name, origin, demonstrates, description, url, homepage) -> dict:
    return {"name": name, "origin": origin, "demonstrates": demonstrates.strip(),
            "description": description.strip(), "url": url, "homepage": homepage}


def entries_from_toml(text: str, meta: dict | None, origin: str) -> list[dict]:
    """Parse a `.loft-showcase.toml` ([[showcase]] tables). `meta` (repo name/description/url/
    homepage) fills the gaps for a standalone repo; None for in-repo demos (fields are explicit)."""
    try:
        data = tomllib.loads(text)
    except tomllib.TOMLDecodeError:
        return []
    out = []
    for e in data.get("showcase", []):
        out.append(entry(
            name=e.get("name") or (meta or {}).get("name", ""),
            origin=origin,
            demonstrates=e.get("demonstrates", ""),
            description=e.get("description") or (meta or {}).get("description", ""),
            url=e.get("url") or (meta or {}).get("url", ""),
            homepage=e.get("demo") or (meta or {}).get("homepage", ""),
        ))
    return out


def local_loft_demos() -> list[dict]:
    if not LOCAL_DESCRIPTOR.exists():
        return []
    return entries_from_toml(LOCAL_DESCRIPTOR.read_text(encoding="utf-8"), None, "first-party")


def topic_repos() -> list[dict]:
    """First-party standalone apps: repos with the `loft-showcase` topic. Read each repo's
    `.loft-showcase.toml` if present (self-described tagline); else fall back to repo metadata
    (a repo may be branch-protected and unable to hold the file)."""
    out = []
    for owner in FIRST_PARTY_OWNERS:
        repos = json.loads(gh(["repo", "list", owner, "--topic", TOPIC, "--no-archived",
                               "--json", "name,description,url,homepageUrl", "--limit", "100"]) or "[]")
        for r in repos:
            meta = {"name": r["name"], "description": (r.get("description") or "").strip(),
                    "url": r.get("url", ""), "homepage": (r.get("homepageUrl") or "").strip()}
            b64 = gh(["api", f"repos/{owner}/{r['name']}/contents/.loft-showcase.toml", "--jq", ".content"]).strip()
            parsed = entries_from_toml(base64.b64decode(b64).decode(), meta, "first-party") if b64 else []
            out += parsed or [entry(meta["name"], "first-party", "", meta["description"], meta["url"], meta["homepage"])]
    return out


def parse_form(body: str):
    sections, cur, buf = {}, None, []
    for line in (body or "").splitlines():
        if line.startswith("### "):
            if cur is not None:
                sections[cur] = "\n".join(buf).strip()
            cur, buf = line[4:].strip(), []
        elif cur is not None:
            buf.append(line)
    if cur is not None:
        sections[cur] = "\n".join(buf).strip()

    def get(prefix: str) -> str:
        for k, v in sections.items():
            if k.lower().startswith(prefix.lower()):
                return "" if v.strip() in ("_No response_", "") else v.strip()
        return ""
    return get


def community_issues() -> list[dict]:
    issues = json.loads(gh(["issue", "list", "--repo", ISSUE_REPO, "--label", LIST_LABEL,
                            "--state", "open", "--limit", "200", "--json", "number,title,body,url"]) or "[]")
    out = []
    for iss in issues:
        get = parse_form(iss.get("body") or "")
        if not get("What it demonstrates"):
            continue
        repo_field = get("Public repository").strip()
        url = repo_field if repo_field.startswith("http") else (f"https://github.com/{repo_field}" if repo_field else iss["url"])
        out.append(entry(
            name=get("App name") or iss.get("title", "").removeprefix("[showcase]").strip(),
            origin="community",
            demonstrates=get("What it demonstrates"),
            description=get("One-line summary"),
            url=url,
            homepage=get("Live demo"),
        ))
    return out


def main() -> int:
    apps = local_loft_demos() + topic_repos() + community_issues()
    apps.sort(key=lambda a: (a["origin"] != "first-party", a["name"].lower()))
    OUT.write_text(json.dumps({"applications": apps}, indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")
    fp = sum(1 for a in apps if a["origin"] == "first-party")
    print(f"refresh-applications: wrote {OUT} ({len(apps)} apps — {fp} first-party self-described, {len(apps) - fp} community)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
