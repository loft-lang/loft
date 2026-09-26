#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Install this repo's Claude Code hooks into the local `.claude/settings.json`.

`.claude/settings.json` is per-machine and git-ignored, so a hook the repo wants every agent
to run cannot live in it under version control.  This script is how it travels: `make hooks`
runs it once per clone, and it MERGES — an entry already present is left alone, and every
other setting and hook in the file is kept.  Running it twice changes nothing.

The hooks it installs:

    PostToolUse  Edit|Write|MultiEdit   scripts/doc_lint.py --hook
                 the documentation lint on the file just written: silent when the edit
                 added nothing, the new findings as context when it did (DOC_CONTRACT.md)
"""
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SETTINGS = os.path.join(ROOT, ".claude", "settings.json")

HOOKS = [
    ("PostToolUse", "Edit|Write|MultiEdit",
     {"type": "command",
      "command": 'python3 "$CLAUDE_PROJECT_DIR/scripts/doc_lint.py" --hook 2>/dev/null || true',
      "timeout": 10}),
]


def main() -> int:
    try:
        with open(SETTINGS, encoding="utf-8") as f:
            settings = json.load(f)
    except FileNotFoundError:
        settings = {}
    except ValueError as e:
        print(f"install_claude_hooks: {SETTINGS} is not valid JSON ({e}); fix it first",
              file=sys.stderr)
        return 1
    added = 0
    for event, matcher, hook in HOOKS:
        groups = settings.setdefault("hooks", {}).setdefault(event, [])
        group = next((g for g in groups if g.get("matcher") == matcher), None)
        if group is None:
            group = {"matcher": matcher, "hooks": []}
            groups.append(group)
        if not any(h.get("command") == hook["command"] for h in group.setdefault("hooks", [])):
            group["hooks"].append(hook)
            added += 1
    if added:
        os.makedirs(os.path.dirname(SETTINGS), exist_ok=True)
        with open(SETTINGS, "w", encoding="utf-8") as f:
            json.dump(settings, f, indent=2)
            f.write("\n")
        print(f"install_claude_hooks: {added} hook(s) added to .claude/settings.json "
              "(open /hooks once, or restart, for a running session to pick them up)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
