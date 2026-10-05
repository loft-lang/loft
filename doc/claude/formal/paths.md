<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/paths.md — what a path means to a loft program (strict)

**Catalogue:** @F40 (file & directory I/O), @PLN184 (loft fully functional on Windows).

> **Rules then deviations** (see [README](README.md)).  A loft program reaches the file system
> only through the stdlib's path-taking functions (`file`, `exists`, `list_dir`, `write_bytes`,
> `delete`, `move`, `mkdir*`, `rmdir`, the `store_*` loaders) and is handed paths back by a few
> (`temp_dir`, `cache_dir`, `files`, `path_sep`).  The rules make one program answer the same on
> every platform: what one platform cannot do, no platform does.

## The model in one line

A loft path is names separated by `/` (or `\`, read as `/`); a name means exactly its spelling; a path no platform could
hold is refused everywhere; a path loft hands out is in that same form.

## Rules

```
  (Path-Sep)    A path a program HANDS loft may separate with `/` or `\` on every platform —
                a Windows user's `C:\data\x.txt` argument works, and means the same on Linux
                (no name may hold a `\`, by Path-Name, so nothing is lost).  A path loft GIVES
                a program (`temp_dir`, `cache_dir`, `files()`'s paths) uses `/` only and carries
                no trailing separator; `path_sep()` answers '/'.  On Windows an absolute path is
                handed out as `C:/…`.

  (Path-Name)   A name that is illegal on ANY platform is refused on EVERY platform:
                a control character or one of  < > : " \ | ? * ,  a device name CON PRN AUX NUL
                COM1–COM9 LPT1–LPT9 (any case, with or without an extension), or a name ending
                in `.` or a space (`.` and `..` themselves are names of the walk, not refused).
                The one `:` allowed is a drive prefix `C:` leading an absolute path.

  (Path-Case)   A name means exactly its spelling.  A name that matches an entry of its
                directory only when case is ignored is refused — for reading (it does not
                exist) and for writing (it is not created) — so `b.txt` and `B.txt` are never
                one file on one platform and two on another.

  (Path-Refuse) A refused path is the operation's own failure answer — `exists` false,
                `list_dir` / `content` / `read_bytes` null, `write` / `delete` / `move` /
                `mkdir` FileResult.Other — and one log line naming the path and the rule.
                Never a runtime error (C80).
```

**In words.**  loft programs are meant to run unchanged wherever loft runs.  A path is the one
place the host leaks in by default: Windows has a second separator, refuses names Linux allows,
and folds case.  Each rule picks the answer that is possible on every platform, so a program
that works on Linux works on Windows, and one that fails, fails the same way.

*Anchors:* `Stores::resolve_path` (`src/database/mod.rs`) — the one home every path-taking
operation of both backends routes through — and `file_access::program_path`
(`src/file_access/mod.rs`); `tests/scripts/a-program-cannot-tell-which-platform-it-runs-on.loft`.

## Deviations

**OPEN: 0.**  The four this chapter opened with — the Windows separator and directories,
names one platform takes and another refuses, case folding, and `\` meaning two things — are
closed; the record is in [paths-history.md](paths-history.md).
