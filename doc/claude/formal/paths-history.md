<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/paths.md — history

The closed deviations of [paths.md](paths.md).

## Closed 2026-10-02 (@PLN184, measured on windows-latest)

The chapter was written against a measurement: `tests/scripts/a-program-cannot-tell-which-
platform-it-runs-on.loft` passed 8 of 8 on Linux and failed 6 of 8 on windows-latest, on both
backends (windows-probe run 37060992821, commit 08b0ad037).

```
  D-path-1  (Path-Sep)   On Windows `path_sep()` answered '\', and `temp_dir()` / `cache_dir()`
                         `C:\…\` with backslashes and a trailing separator.
                         CLOSED: every directory loft gives goes through `file_access::given`.
  D-path-2  (Path-Name)  Each platform wrote the names its OS took: on Windows `a:b.txt`
                         silently wrote an NTFS alternate data stream; on Linux `aux.txt`,
                         `a:b.txt`, `q?.txt`, `trail.` were all written.
                         CLOSED: `PathText::program` refuses the union, on every platform.
  D-path-3  (Path-Case)  On Windows `exists("B.TXT")` found `b.txt`, and writing `B.txt`
                         overwrote it.
                         CLOSED: `file_access::case_clash`, asked by `Stores::resolve_path`.
  D-path-4  (Path-Sep)   On Linux a `\` was part of a name, where Windows read it as a
                         separator.
                         CLOSED: `\` separates on every host.
```

After the fix (commit 832ee4ccd), the guard passes 9 of 9 on both backends on Linux and on
windows-latest (windows-probe run 37064377332), with a UTF-8 round-trip row added.

## Closed 2026-10-07 (@PLN184 U2)

```
  D-path-5  (Path-Utf8)  A listing left a non-UTF-8 name out, so the program could not see the
                         entry; and the U+FFFD spelling of such a name reached the file system as
                         that spelling, so a write created a second entry that listed the same.
                         CLOSED: `file_access::read_dir` lists the entry (U+FFFD, one log line per
                         directory) and `PathText` keeps the OS's own spelling of the name;
                         `file_access::case_clash` refuses a spelling that names an entry only
                         through U+FFFD.  Guard: `windows_rules::a_name_that_is_not_text_is_listed_
                         and_never_reached` — with the refusal removed, `write` and `delete` of the
                         U+FFFD spelling answer true.
```
