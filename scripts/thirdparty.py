#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The third-party code loft and its libraries ship: what it is, where it came from, what is known against it.

Every package declares what it ships or loads that it did not write, in its manifest — a
library in its `loft.toml`, loft itself in `thirdparty.toml` at the repository root — as one
`[thirdparty.<name>]` table per dependency (doc/claude/THIRDPARTY.md):

    [thirdparty.stb_vorbis]
    kind       = "vendored"                       # source copied into the package
    path       = "c/vendor/stb_vorbis"            # relative to the manifest
    upstream   = "https://github.com/nothings/stb"
    commit     = "2c980bb59875b0d32144a71867fbdebb2f77cd20"
    version    = "1.22"
    digest     = "sha256:…"                       # `thirdparty.py digest <path>`
    advisories = ["nvd:stb_vorbis", "osv:commit"]

    [thirdparty.stb_vorbis.reviewed]              # one line per advisory a register reports
    CVE-2019-13217 = "fixed: in v1.17, before our pin"

    [thirdparty.alsa]
    kind     = "system"                           # linked or opened at run time, never shipped
    names    = ["asound"]                         # matched against link flags and sonames
    provider = "the OS sound stack; miniaudio opens it when present"

The audit then checks each declaration against the tree and asks the registers:

  * TREE — code with a foreign copyright notice or under a `vendor/` directory must lie under a
    declared `vendored` path; every library the package links or opens (`[c] libs`,
    `optional-libs`, `[native] runtime-libs`, a soname or DLL literal in its C, loft's own `-l`
    flags) must be named by a `system` entry; a vendored path must still hash to its `digest`.
  * UPSTREAM — every vendored file must be byte-identical to a file of that name at `commit`
    (`renames` maps ours → theirs, `modified` lists the files allowed to differ), and upstream
    commits since the pin that touch one of our files are reported, until `upstream-reviewed`
    names the upstream commit they were read up to.
  * REGISTERS — `nvd:<keyword>` (NVD's CVE search), `osv:commit` (OSV.dev, by the pinned
    commit), `github:<owner>/<repo>` (the repository's security advisories).  Every advisory
    they return needs a `reviewed` line whose verdict is `fixed`, `not-reached` or `applies`;
    an unreviewed one, or one that `applies`, needs action.
  * RUST — every `Cargo.lock` shipped goes through `cargo audit`, declared by a `cargo` entry
    (`lockfile = [...]`); each vulnerability, and each unmaintained, unsound or yanked crate,
    needs a verdict in that entry's `reviewed` table.  Pending patch updates are reported.

What is audited: loft is this checkout's working tree; every library is the latest release
tarball the registry index lists (what `loft install` hands out), or `--source main` for each
library repository's `main`.  Network answers are cached for a day under
~/.cache/loft/thirdparty (`--refresh` asks again).

Usage:
    scripts/thirdparty.py                 # loft + every published library: inventory and status
    scripts/thirdparty.py --check         # silent unless something needs action (exit 1)
    scripts/thirdparty.py --no-libs       # loft only
    scripts/thirdparty.py --repo DIR      # one local tree instead (a library checkout)
    scripts/thirdparty.py --offline       # the TREE checks only: no upstream, no registers
    scripts/thirdparty.py --gate [--repo DIR] [--binary PATH]
                                          # the CI / publish gate: undeclared anything fails it
    scripts/thirdparty.py digest DIR      # the digest line a declaration records
    scripts/thirdparty.py identify DIR URL  # the upstream commit(s) whose files DIR matches
"""
import argparse
import fnmatch
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CACHE = os.environ.get("LOFT_THIRDPARTY_CACHE") or os.path.join(
    os.environ.get("XDG_CACHE_HOME") or os.path.expanduser("~/.cache"), "loft", "thirdparty")
REGISTRY_INDEX = "https://raw.githubusercontent.com/loft-lang/registry/main/index.json"
REGISTRY_REPO = "https://github.com/loft-lang/registry"
DAY = 24 * 3600
# How old a register's last answer may be when it does not answer now (`http`).
STALE_OK = 7 * DAY
STALE_ANSWERS = []
CODE_EXT = (".c", ".h", ".cc", ".cpp", ".hpp", ".inl")
# Code a browser, GL or wasm target ships beside the C: scanned for notices too.
SCAN_EXT = CODE_EXT + (".js", ".mjs", ".css", ".wgsl", ".glsl", ".wat")
# The platforms a Rust dependency is resolved for, and the target_os values they are.
SHIPPED_TARGETS = ("x86_64-unknown-linux-gnu", "aarch64-apple-darwin", "x86_64-pc-windows-msvc")
SHIPPED_OS = {"linux", "macos", "windows"}
INCLUDE_LIT = re.compile(r'include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)')
RUST_LINK = re.compile(r'\blink\(\s*name\s*=\s*"([^"]+)"')   # #[link(..)] and cfg_attr(.., link(..))
# A source file named for a platform loft does not ship (getrandom's fuchsia.rs) is that platform's.
OTHER_OS_FILE = re.compile(r"(fuchsia|redox|hermit|solaris|illumos|haiku|vxworks|horizon|espidf|"
                           r"android|ios|tvos|watchos|freebsd|netbsd|openbsd|dragonfly|aix|nto|zkvm|"
                           r"emscripten|cygwin)", re.I)
BUILD_C = re.compile(r"\bcc::|\bcmake::|Build::new\(\)")
# Path segments whose code is not part of what a package ships.
NOT_SHIPPED = {"tests", "test", "doc", "docs", "examples", "bench", "target", ".loft",
               "node_modules", "fixtures", "native-auto", ".git", "tools"}
VERDICTS = ("fixed", "not-reached", "applies")
# What an upstream commit to one of our files is, read from its subject — the update policy's
# input (THIRDPARTY.md § When to update).  A heuristic over words, so the report shows the
# subjects it counted and a person reads them before acting.
SECURITY_MSG = re.compile(r"(?i)(\bcve\b|security|overflow|out[- ]of[- ]bounds|\boob\b|use[- ]after|"
                          r"double[- ]free|\bcrash|segfault|\bleak|sanitiz|undefined behaviou?r|\brace\b|deadlock)")
FIX_MSG = re.compile(r"(?i)(\bfix|\bbug|\berror\b|wrong|incorrect|regression|\bassert|\bnull\b|\bhang)")
TRIVIAL_MSG = re.compile(r"(?i)^(typo|spelling|comment|doc|readme|whitespace|formatting|style|version \d)")
PERF_MSG = re.compile(r"(?i)(\bperf|faster|speed|optimi[sz]|fast[- ]path|\bsimd\b|\bsse|\bavx|\bneon\b|vectori[sz])")
TIERS = {}   # name → (tier, why), for the closing summary
# The platform C library and compiler runtime are in every process: binding to them needs no
# entry (PACKAGES_C_BINDING.md § `libs`), so a library is never asked to declare them.
PLATFORM_C = {"c", "m", "dl", "pthread", "rt", "gcc_s", "ld-linux-x86-64", "ld-linux-aarch64", "system"}
OWN_NOTICE = re.compile(r"Jurjen Stellingwerff|LGPL-3\.0-or-later")
FOREIGN_NOTICE = re.compile(r"copyright|public domain|permission is hereby granted|"
                            r"spdx-license-identifier", re.I)
SONAME_LIT = re.compile(r'"((?:/[^"\s]*/)?lib[A-Za-z0-9_+-]+(?:\.[A-Za-z0-9_]+)*?'
                        r'\.(?:so(?:\.\d+)*|dylib))"')
FRAMEWORK_LIT = re.compile(r'"(/System/Library/Frameworks/[A-Za-z0-9]+\.framework/[A-Za-z0-9]+)"')
PRAGMA_LIB = re.compile(r'#\s*pragma\s+comment\s*\(\s*lib\s*,\s*"([^"]+)"')
DLL_LIT = re.compile(r'"([A-Za-z0-9_]+\.dll)"', re.I)
LFLAG_LIT = re.compile(r'"(-l:?[A-Za-z0-9_.+-]+|-pthread)"')
TEST_MOD = re.compile(r"#\[cfg\(test\)\]\s*(pub(\([^)]*\))?\s+)?mod \w+\s*\{")
# What a build fetches and compiles into what it ships: an ESP component-manager dependency
# (`badgeteam/badge-bsp: "=1.6.4"`) and a repository a setup step clones.
COMPONENT_LIT = re.compile(r'([a-z0-9_-]+/[a-z0-9_-]+): \\"[=^~<>]*\d[\w.]*\\"')
CLONE_LIT = re.compile(r'git clone[^"]*?(https://[^\s"]+?)(?:\.git)?[\s"]')
REFRESH = False
OFFLINE = False
GATE = False
BINARY = None


# ── small helpers ────────────────────────────────────────────────────────────────────────────

def run(cmd, cwd=None, check=False):
    r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    if check and r.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd)}: {r.stderr.strip()[-300:]}")
    return r


def touch(path):
    with open(path, "a", encoding="utf-8"):
        pass
    os.utime(path)


def git_blob_id(path):
    with open(path, "rb") as f:
        data = f.read()
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def lib_base(name):
    """`-lGL`, `libGL.so.1`, `/usr/lib/libobjc.A.dylib`, `ws2_32.dll`, an AppKit framework path
    → the one name a `system` entry lists: `GL`, `objc`, `ws2_32`, `AppKit`."""
    s = name.strip()
    if s == "-pthread":
        return "pthread"
    if s.startswith("-l"):
        s = s[2:].lstrip(":")
    s = os.path.basename(s)
    s = re.sub(r"\.(so(\.\d+)*|dylib|dll|lib|a)$", "", s, flags=re.I)
    s = re.sub(r"(\.(\d+|[A-Z]))+$", "", s)   # a framework version letter: libobjc.A, libSystem.B
    if s.startswith("lib") and len(s) > 3:
        s = s[3:]
    return s.lower()


def split_list(v):
    if isinstance(v, list):
        return [str(x).strip() for x in v if str(x).strip()]
    return [x.strip() for x in str(v or "").split(",") if x.strip()]


def posix(p):
    """A tree-relative path in the one spelling the audit compares: `/`-separated, as the file
    list and the manifests write it.  `os.path.normpath` and `os.path.relpath` answer with `\\`
    on Windows, which no `startswith(p + "/")` and no lookup in the file list then matches."""
    return os.path.normpath(p).replace(os.sep, "/")


def shipped(rel):
    return not (set(re.split(r"[\\/]", rel)[:-1]) & NOT_SHIPPED)


def http(url, data=None, headers=None, ttl=DAY):
    """GET/POST with a day-long cache; returns bytes, or raises on a network failure."""
    key = hashlib.sha256((url + "\0" + (data.decode() if data else "")).encode()).hexdigest()
    path = os.path.join(CACHE, "http", key)
    if not REFRESH and os.path.exists(path) and time.time() - os.path.getmtime(path) < ttl:
        with open(path, "rb") as f:
            return f.read()
    if OFFLINE:
        raise RuntimeError("offline")
    req = urllib.request.Request(url, data=data, headers={"User-Agent": "loft-thirdparty-audit",
                                                          **(headers or {})})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            body = r.read()
    except (urllib.error.URLError, OSError) as e:
        # A register that does not answer this time answered before: its last answer, up to
        # STALE_OK old, is what is known, and the report says how old it is.  Older, or never
        # asked, nothing is known and the caller reports that.
        if os.path.exists(path) and time.time() - os.path.getmtime(path) < STALE_OK:
            age = (time.time() - os.path.getmtime(path)) / 3600
            STALE_ANSWERS.append(f"{url.split('?')[0]} did not answer ({e}); its answer of {age:.0f} h ago is used")
            with open(path, "rb") as f:
                return f.read()
        raise
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(body)
    return body


# ── trees ────────────────────────────────────────────────────────────────────────────────────

class Tree:
    """A directory and the files in it that count (git-tracked when it is a checkout)."""

    def __init__(self, label, root, is_loft=False):
        self.label, self.root, self.is_loft = label, root, is_loft
        r = run(["git", "-C", root, "ls-files", "-z"])
        if r.returncode == 0 and os.path.isdir(os.path.join(root, ".git")):
            self.files = [f for f in r.stdout.split("\0") if f and os.path.isfile(os.path.join(root, f))]
        else:
            self.files = []
            for d, dirs, names in os.walk(root):
                dirs[:] = [x for x in dirs if x != ".git"]
                for n in names:
                    self.files.append(posix(os.path.relpath(os.path.join(d, n), root)))
        self.files.sort()


def fetch_tarball(url, dest, sha256=None):
    if not os.path.isdir(dest) or REFRESH:
        body = http(url, ttl=365 * DAY)
        if sha256 and hashlib.sha256(body).hexdigest() != sha256:
            raise RuntimeError(f"{url}: sha256 differs from the registry index")
        shutil.rmtree(dest, ignore_errors=True)
        os.makedirs(dest)
        tmp = dest + ".tar.gz"
        with open(tmp, "wb") as f:
            f.write(body)
        with tarfile.open(tmp) as t:
            t.extractall(dest, filter="data")
        os.remove(tmp)
    return dest


def registry_index():
    """The index at the registry's current commit: the branch address is CDN-cached apart from
    the commit, and can serve the previous index for minutes after a publish."""
    head = run(["git", "ls-remote", REGISTRY_REPO, "refs/heads/main"]).stdout.split()
    if head:
        return json.loads(http(REGISTRY_INDEX.replace("/main/", f"/{head[0]}/"), ttl=365 * DAY))
    return json.loads(http(REGISTRY_INDEX))


def registry_trees(source):
    index = registry_index()
    trees, repos = [], {}
    for name, p in sorted(index["packages"].items()):
        repo = re.search(r"github\.com/([\w.-]+/[\w.-]+)", p.get("homepage", "") + json.dumps(p))
        if name == "loft":
            continue
        if source == "main":
            if repo:
                repos.setdefault(repo.group(1), []).append(name)
            continue
        yanked = set(p.get("yanked", []))
        vers = [v for v in p.get("versions", {}) if v not in yanked]
        if not vers:
            continue
        ver = max(vers, key=lambda v: [int(x) if x.isdigit() else 0 for x in re.split(r"[.+-]", v)])
        e = p["versions"][ver]
        dest = os.path.join(CACHE, "pkgs", f"{name}-{ver}")
        trees.append((f"{name} {ver}", lambda e=e, dest=dest: fetch_tarball(e["url"], dest, e.get("sha256"))))
    for repo, names in sorted(repos.items()):
        sha = run(["git", "ls-remote", f"https://github.com/{repo}", "refs/heads/main"]).stdout.split()
        sha = sha[0] if sha else "main"
        dest = os.path.join(CACHE, "repos", repo.replace("/", "__") + "@" + sha[:12])
        url = f"https://codeload.github.com/{repo}/tar.gz/{sha}"
        trees.append((f"{repo.rpartition('/')[2]} @ main {sha[:9]}", lambda url=url, dest=dest: fetch_tarball(url, dest)))
    return trees


# ── upstream (blob-less bare clones) ─────────────────────────────────────────────────────────

def upstream_clone(url):
    path = os.path.join(CACHE, "upstream", re.sub(r"[^A-Za-z0-9]+", "_", url) + ".git")
    stamp = os.path.join(path, "loft-fetched")
    if os.path.isdir(path):
        if REFRESH or not os.path.exists(stamp) or time.time() - os.path.getmtime(stamp) > DAY:
            if not OFFLINE:
                run(["git", "-C", path, "fetch", "-q", "--tags", "--force", "origin",
                     "+refs/heads/*:refs/heads/*"])
                touch(stamp)
        return path
    if OFFLINE:
        raise RuntimeError("offline")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    r = run(["git", "clone", "-q", "--bare", "--filter=blob:none", url, path])
    if r.returncode != 0:   # a dumb-http server has no partial clone
        shutil.rmtree(path, ignore_errors=True)
        run(["git", "clone", "-q", "--bare", url, path], check=True)
    touch(stamp)
    return path


def upstream_blobs(clone, commit):
    out = run(["git", "-C", clone, "ls-tree", "-r", commit], check=True).stdout
    by = {}
    for line in out.splitlines():
        meta, p = line.split("\t", 1)
        by.setdefault(os.path.basename(p), {})[meta.split()[2]] = p
    return by


def identify(dirpath, url):
    clone = upstream_clone(url)
    ours = {f: git_blob_id(os.path.join(dirpath, f)) for f in os.listdir(dirpath) if f.endswith(CODE_EXT)}
    best = []
    for c in run(["git", "-C", clone, "rev-list", "--all"], check=True).stdout.split():
        by = upstream_blobs(clone, c)
        miss = sorted(f for f, b in ours.items() if b not in by.get(f, {}))
        best.append((len(miss), c, miss))
    best.sort(key=lambda x: x[0])
    for n, c, miss in best[:5]:
        d = run(["git", "-C", clone, "log", "-1", "--format=%ad %s", "--date=short", c]).stdout.strip()
        desc = run(["git", "-C", clone, "describe", "--tags", c]).stdout.strip()
        print(f"{n:4d} of {len(ours)} differ  {c}  {desc or '-'}  {d[:60]}  {' '.join(miss[:5])}")


def tree_digest(dirpath):
    h = hashlib.sha256()
    for d, dirs, names in sorted(os.walk(dirpath)):
        dirs.sort()
        for n in sorted(names):
            if n.endswith(CODE_EXT):
                p = os.path.join(d, n)
                with open(p, "rb") as f:
                    h.update(f"{posix(os.path.relpath(p, dirpath))}\0{hashlib.sha256(f.read()).hexdigest()}\n".encode())
    return "sha256:" + h.hexdigest()


# ── registers ────────────────────────────────────────────────────────────────────────────────

_last_nvd = [0.0]


def reg_nvd(keyword):
    out, start = {}, 0
    while True:
        url = ("https://services.nvd.nist.gov/rest/json/cves/2.0?"
               + urllib.parse.urlencode({"keywordSearch": keyword, "resultsPerPage": 2000, "startIndex": start}))
        key = os.environ.get("NVD_API_KEY")
        wait = 0.7 if key else 6.5   # NVD's public rate limit: 5 requests in 30 s
        cached = os.path.join(CACHE, "http", hashlib.sha256((url + "\0").encode()).hexdigest())
        if not (os.path.exists(cached) and time.time() - os.path.getmtime(cached) < DAY) or REFRESH:
            time.sleep(max(0.0, _last_nvd[0] + wait - time.time()))
            _last_nvd[0] = time.time()
        d = json.loads(http(url, headers={"apiKey": key} if key else None))
        for v in d.get("vulnerabilities", []):
            c = v["cve"]
            desc = next((x["value"] for x in c.get("descriptions", []) if x.get("lang") == "en"), "")
            out[c["id"]] = (c.get("published", "")[:10], desc)
        start += d.get("resultsPerPage", 0)
        if start >= d.get("totalResults", 0) or not d.get("resultsPerPage"):
            return out


def reg_osv_commit(commit):
    out, token = {}, None
    while True:
        q = {"commit": commit, **({"page_token": token} if token else {})}
        d = json.loads(http("https://api.osv.dev/v1/query", data=json.dumps(q).encode(),
                            headers={"Content-Type": "application/json"}))
        for v in d.get("vulns", []):
            cve = next((a for a in v.get("aliases", []) if a.startswith("CVE-")), None)
            ident = v["id"] if v["id"].startswith("CVE-") else cve or v["id"]
            out[ident] = (v.get("published", "")[:10], v.get("summary") or v.get("details", ""))
        token = d.get("next_page_token")
        if not token:
            return out


def reg_github(repo):
    if OFFLINE:
        raise RuntimeError("offline")
    r = run(["gh", "api", "--paginate", f"repos/{repo}/security-advisories"])
    if r.returncode != 0:
        raise RuntimeError(r.stderr.strip()[-200:])
    out = {}
    for a in json.loads(r.stdout or "[]"):
        ranges = [v.get("vulnerable_version_range") for v in a.get("vulnerabilities") or []]
        out[a.get("cve_id") or a["ghsa_id"]] = ((a.get("published_at") or "")[:10], a.get("summary", ""),
                                                [x for x in ranges if x])
    return out


def vkey(v):
    return [int(x) for x in re.findall(r"\d+", str(v))[:4]]


def in_range(version, rng):
    """Does `version` satisfy a GitHub range such as `>= 5.0, < 5.0.7` (alternatives by `||`)?
    None when a part cannot be read, so the advisory stays for a person."""
    v = vkey(version)
    if not v:
        return None
    for alt in rng.split("||"):
        ok = True
        for part in alt.split(","):
            m = re.match(r"\s*(<=|>=|<|>|=)?\s*v?([\d.]+)\s*$", part)
            if not m:
                return None
            op, w = m.group(1) or "=", vkey(m.group(2))
            n = max(len(v), len(w))
            a, b = v + [0] * (n - len(v)), w + [0] * (n - len(w))
            ok &= {"<": a < b, "<=": a <= b, ">": a > b, ">=": a >= b, "=": a == b}[op]
        if ok:
            return True
    return False


# ── the audit ────────────────────────────────────────────────────────────────────────────────

class Audit:
    def __init__(self):
        self.action = []   # (where, what)
        self.lines = []    # the inventory, printed unless --check

    def need(self, where, what):
        self.action.append((where, what))

    def say(self, s):
        self.lines.append(s)


def read_manifest(path):
    with open(path, "rb") as f:
        return tomllib.load(f)


def packages_of(tree):
    """[(pkgdir_rel, manifest dict)] — every shipped `loft.toml`, plus loft's `thirdparty.toml`."""
    pk = [("", "thirdparty.toml")] if tree.is_loft else []
    for f in tree.files:
        if os.path.basename(f) == "loft.toml" and shipped(f) and not (tree.is_loft and f == "loft.toml"):
            pk.append((os.path.dirname(f), f))
    return pk


def owner_pkg(rel, pkgdirs):
    best = None
    for d in pkgdirs:
        if d == "" or rel.startswith(d + "/"):
            if best is None or len(d) > len(best):
                best = d
    return best


def block_end(text, i):
    """The index just past the `}` closing the `{` at `i`, skipping strings, chars and comments."""
    depth, n = 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            i = text.find("\n", i)
            i = n if i < 0 else i
        elif text.startswith("/*", i):
            i = text.find("*/", i) + 2 or n
        elif c == "r" and re.match(r'r#*"', text[i:i + 8]) and not text[i - 1:i].isalnum():
            hashes = re.match(r'r(#*)"', text[i:]).group(1)
            i = text.find('"' + hashes, i + len(hashes) + 2) + len(hashes) + 1
        elif c == '"':
            i += 1
            while i < n and text[i] != '"':
                i += 2 if text[i] == "\\" else 1
        elif c == "'" and re.match(r"'(\\.|[^\\'])'", text[i:i + 4]):
            i = text.find("'", i + 2)
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    return n


def strip_test_mods(text):
    out, pos = [], 0
    for m in TEST_MOD.finditer(text):
        if m.start() < pos:
            continue
        out.append(text[pos:m.start()])
        pos = block_end(text, m.end() - 1)
    out.append(text[pos:])
    return "".join(out)


def loft_rust_source(tree):
    for f in tree.files:
        if f.startswith("src/") and f.endswith(".rs"):
            with open(os.path.join(tree.root, f), encoding="utf-8", errors="replace") as fh:
                yield f, strip_test_mods(fh.read())


def loft_link_flags(tree):
    """The `-l` flags loft's own C driver passes, from its Rust source (test modules excluded)."""
    found = {}
    for f, text in loft_rust_source(tree):
        for m in LFLAG_LIT.finditer(text):
            found.setdefault(m.group(1), f)
    return found


def loft_sdk_pins(tree):
    """{source: file} — what loft's build steps fetch and compile in (an ESP component, a clone)."""
    found = {}
    for f, text in loft_rust_source(tree):
        for m in COMPONENT_LIT.finditer(text):
            found.setdefault("esp-registry:" + m.group(1), f)
        for m in CLONE_LIT.finditer(text):
            found.setdefault(m.group(1).rstrip("/"), f)
    return found


def loft_embedded(tree):
    """{file: includer} — every file loft's Rust source embeds with include_str!/include_bytes!:
    it ships inside the binary wherever it lives in the tree (doc/, index/, src/c/)."""
    found = {}
    for f, text in loft_rust_source(tree):
        for m in INCLUDE_LIT.finditer(text):
            rel = posix(os.path.join(os.path.dirname(f), m.group(1)))
            if os.path.isfile(os.path.join(tree.root, rel)):
                found.setdefault(rel, f)
    return found


def pe_imports(path):
    """The DLLs a Windows PE binary imports, read from its import directory."""
    import struct
    with open(path, "rb") as f:
        b = f.read()
    if b[:2] != b"MZ":
        return []
    pe = struct.unpack_from("<I", b, 0x3C)[0]
    nsec, optsz = struct.unpack_from("<H", b, pe + 6)[0], struct.unpack_from("<H", b, pe + 20)[0]
    opt = pe + 24
    dd = opt + (112 if struct.unpack_from("<H", b, opt)[0] == 0x20B else 96)
    imp_rva = struct.unpack_from("<I", b, dd + 8)[0]
    secs = [struct.unpack_from("<8sIIII", b, opt + optsz + 40 * i) for i in range(nsec)]

    def off(rva):
        for _, vsize, va, rsize, raw in secs:
            if va <= rva < va + max(vsize, rsize):
                return rva - va + raw
        return None

    out, o = [], off(imp_rva) if imp_rva else None
    while o is not None and o + 20 <= len(b):
        name_rva = struct.unpack_from("<I", b, o + 12)[0]
        if not name_rva:
            break
        no = off(name_rva)
        if no is None:
            break
        out.append(b[no:b.index(b"\0", no)].decode("ascii", "replace"))
        o += 20
    return out


def binary_needed(path):
    """The libraries a built binary is linked against — ELF (`readelf -d`), Mach-O (`otool -L`)
    or PE (its import directory) — so the source scan is checked against the artefact."""
    for cand in (path, path + ".exe"):
        if os.path.exists(cand):
            path = cand
            break
    else:
        return []
    with open(path, "rb") as f:
        magic = f.read(4)
    if magic == b"\x7fELF" and shutil.which("readelf"):
        return re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]", run(["readelf", "-d", path]).stdout)
    if magic[:2] == b"MZ":
        return pe_imports(path)
    if shutil.which("otool"):
        lines = run(["otool", "-L", path]).stdout.splitlines()[1:]
        return [ln.strip().split(" (")[0] for ln in lines if ln.strip()]
    return []


INCLUDE_C = re.compile(r'^\s*#\s*include\s*"([^"]+)"', re.M)


def shim_closure(root, d, shim):
    """The [c] shim sources and every file they `#include "…"`, resolved as the compiler would:
    beside the including file, then the package's `c/` and the package itself."""
    done, todo = set(), list(shim)
    while todo:
        f = todo.pop()
        if f in done or not os.path.isfile(os.path.join(root, f)):
            continue
        done.add(f)
        with open(os.path.join(root, f), encoding="utf-8", errors="replace") as fh:
            for inc in INCLUDE_C.findall(fh.read()):
                for base in (os.path.dirname(f), os.path.join(d, "c"), d):
                    cand = posix(os.path.join(base, inc))
                    if os.path.isfile(os.path.join(root, cand)):
                        todo.append(cand)
                        break
    return done


def shim_artifact(root, d, shim):
    """[(library, how)] — build the shim as a shared object and read what it links (`NEEDED`)
    and the library names its strings hold.  Empty when no C compiler is here."""
    cc = shutil.which("cc") or shutil.which("gcc") or shutil.which("clang")
    if not cc or os.name == "nt":
        return []
    out_dir = os.path.join(CACHE, "shim-build")
    os.makedirs(out_dir, exist_ok=True)
    so = os.path.join(out_dir, re.sub(r"[^A-Za-z0-9]+", "_", os.path.join(root, d)) + ".so")
    srcs = [os.path.join(root, f) for f in shim if f.endswith((".c", ".cc", ".cpp"))]
    r = run([cc, "-shared", "-fPIC", "-O0", "-w", "-o", so, *srcs, "-I" + os.path.join(root, d, "c"),
             "-I" + os.path.join(root, d), "-lm"])
    if r.returncode != 0:
        return [("?", f"could not be built to read what it links ({(r.stderr.strip().splitlines() or ['?'])[0][:120]}); it")]
    found = [(lib, "is linked against") for lib in binary_needed(so)]
    strings = run(["strings", "-n", "5", so]).stdout if shutil.which("strings") else ""
    for rx in (r"lib[A-Za-z0-9_+-]+\.so(?:\.\d+)*", r"[A-Za-z0-9_]+\.dll", r"lib[A-Za-z0-9_+-]+(?:\.[A-Z])?\.dylib",
               r"/System/Library/Frameworks/[A-Za-z0-9]+\.framework/[A-Za-z0-9]+"):
        found += [(x, "names") for x in sorted(set(re.findall(rx, strings)))]
    return found


def norm_source(s):
    return re.sub(r"\.git$", "", str(s or "").strip().rstrip("/"))


def audit_tree(tree, a, offline):
    pkgs = packages_of(tree)
    if not pkgs:
        return
    manifests, decls = {}, {}
    for d, f in pkgs:
        try:
            full = os.path.join(tree.root, f)
            manifests[d] = read_manifest(full) if os.path.exists(full) else {}
        except tomllib.TOMLDecodeError as e:
            a.need(f"{tree.label}/{f}", f"the manifest is not TOML the audit can read: {e}")
            manifests[d] = {}
        decls[d] = manifests[d].get("thirdparty", {}) or {}
    pkgdirs = list(manifests)
    root_decls = decls.get("", {}) if tree.is_loft else {}

    def scope(d):
        return {**root_decls, **decls.get(d, {})}

    # What the tree holds: third-party code, and the libraries it links or opens.  For loft the
    # shipped set is exact as well as heuristic: every file its binary embeds counts, wherever
    # it lives, and must be ours by its notice, ours by `[own] paths`, or a declared vendored file.
    embedded = loft_embedded(tree) if tree.is_loft else {}
    own_globs = split_list((manifests.get("", {}).get("own") or {}).get("paths")) if tree.is_loft else []
    foreign, links, unattributed = {}, {}, {}

    def ours(d, head):
        """Our notice, or the one the package names as its author's (`[own] notice`)."""
        mine = str((manifests.get(d, {}).get("own") or {}).get("notice") or "")
        return bool(OWN_NOTICE.search(head)) or (bool(mine) and mine in head)

    for f in tree.files:
        if not (f in embedded or (f.endswith(SCAN_EXT) and shipped(f))):
            continue
        d = owner_pkg(f, pkgdirs)
        if d is None:
            continue
        with open(os.path.join(tree.root, f), encoding="utf-8", errors="replace") as fh:
            text = fh.read()
        head = "\n".join(text.splitlines()[:40])
        if "/vendor/" in "/" + f or (FOREIGN_NOTICE.search(head) and not ours(d, head)):
            foreign.setdefault(d, []).append(f)
        elif not ours(d, head) and (f in embedded or f.startswith("src/c/")) \
                and not any(fnmatch.fnmatch(f, g) for g in own_globs):
            unattributed.setdefault(d, []).append(f)
        if not f.endswith(CODE_EXT):
            continue
        for rx in (SONAME_LIT, FRAMEWORK_LIT, DLL_LIT, PRAGMA_LIB):
            for m in rx.finditer(text):
                links.setdefault(d, {}).setdefault(lib_base(m.group(1)), (m.group(1), f))
    for d, m in manifests.items():
        for key, sect, field in (("[c] libs", "c", "libs"), ("[c] optional-libs", "c", "optional-libs"),
                                 ("[native] runtime-libs", "native", "runtime-libs")):
            for lib in split_list((m.get(sect) or {}).get(field)):
                links.setdefault(d, {}).setdefault(lib_base(lib), (lib, key))
    if tree.is_loft:
        for flag, src in loft_link_flags(tree).items():
            links.setdefault("", {}).setdefault(lib_base(flag), (flag, src))
        # What the built binary is really linked against, when one is here to read.
        built = BINARY or os.path.join(tree.root, "target", "release", "loft")
        for lib in binary_needed(built):
            links.setdefault("", {}).setdefault(lib_base(lib), (lib, f"{os.path.relpath(built, tree.root)} (linked)"))

    cargo_decls, wheres = {}, {}
    for d, m in manifests.items():
        name = (m.get("package") or {}).get("name") or ("loft" if tree.is_loft and d == "" else d)
        where = tree.label if (len(manifests) == 1 or (tree.is_loft and d == "")) else f"{tree.label} · {name}"
        wheres[d] = where
        here = scope(d)
        vendored = {n: x for n, x in here.items() if x.get("kind") == "vendored"}
        system = {n: x for n, x in here.items() if x.get("kind") == "system"}
        for n, x in decls[d].items():
            if x.get("kind") not in ("vendored", "system", "sdk", "cargo"):
                a.need(where, f"{n}: `kind` must be \"vendored\", \"system\", \"sdk\" or \"cargo\"")
            elif x.get("kind") == "cargo":
                for lf in split_list(x.get("lockfile")) or ["?"]:
                    cargo_decls[posix(os.path.join(d, lf))] = (n, x, where)
        if tree.is_loft and d == "":
            sources = {norm_source(x.get("source")) for x in here.values() if x.get("kind") == "sdk"}
            for src, f in sorted(loft_sdk_pins(tree).items()):
                if norm_source(src) not in sources:
                    a.need(where, f"{f} fetches `{src}` into what it builds; no `sdk` entry declares it")

        # Third-party code outside every declared vendored path.
        paths = [posix(os.path.join(d, x.get("path", ""))) for x in vendored.values()]
        loose = [f for f in foreign.get(d, []) if not any(f.startswith(p + "/") for p in paths)]
        for g in sorted({os.path.dirname(f) for f in loose}):
            nf = sum(1 for f in loose if os.path.dirname(f) == g)
            a.need(where, f"{g}/ holds third-party code ({nf} files) no `vendored` entry declares")
        bare = [f for f in unattributed.get(d, []) if not any(f.startswith(p + "/") for p in paths)]
        for g in sorted({os.path.dirname(f) for f in bare}):
            fs = [os.path.basename(f) for f in bare if os.path.dirname(f) == g]
            a.need(where, f"{g}/ ships {len(fs)} file(s) with no notice saying whose they are "
                   f"({', '.join(fs[:4])}{' …' if len(fs) > 4 else ''}): give each our licence header, "
                   "list it in `[own] paths`, or declare it vendored")

        # Libraries linked or opened that no `system` entry names.
        named = {lib_base(x) for s in system.values() for x in split_list(s.get("names"))} | PLATFORM_C
        unnamed = {}
        for base, (lit, src) in sorted(links.get(d, {}).items()):
            if base not in named:
                unnamed.setdefault(src, []).append(base)
        for src, bases in sorted(unnamed.items()):
            a.need(where, f"{src} links or opens {', '.join(bases)}; no `system` entry names "
                   + ("it" if len(bases) == 1 else "them"))

        # A library's [c] shim is exact: every source it lists and every file those include is
        # compiled into the package, so each must be ours by notice, by `[own] paths`, or vendored.
        shim = [posix(os.path.join(d, x)) for x in split_list((m.get("c") or {}).get("shim"))]
        if shim and not tree.is_loft:
            pkg_own = [posix(os.path.join(d, g)) for g in split_list((m.get("own") or {}).get("paths"))]
            for f in sorted(shim_closure(tree.root, d, shim)):
                if any(f.startswith(p + "/") for p in paths) or any(fnmatch.fnmatch(f, g) for g in pkg_own) \
                        or f in loose:
                    continue
                with open(os.path.join(tree.root, f), encoding="utf-8", errors="replace") as fh:
                    head = "\n".join(fh.read().splitlines()[:40])
                if not ours(d, head):
                    a.need(where, f"{f} is compiled into the package ([c] shim) with no notice saying whose "
                           "it is: give it our licence header, list it in `[own] paths`, or declare it vendored")
            # What the built shim really links and names: string concatenation and macros hide a
            # library name from the source scan, never from the object.
            seen_src = set(links.get(d, {}))
            missed = {}
            for lib, why in shim_artifact(tree.root, d, shim):
                if lib == "?":
                    a.need(where, f"its [c] shim {why}")
                elif lib_base(lib) not in named and lib_base(lib) not in seen_src:
                    missed.setdefault(lib_base(lib), why)
            if missed:
                a.need(where, f"its built [c] shim links or names {', '.join(sorted(missed))}, which the "
                       "source scan did not see; no `system` entry names them")
        if not decls[d] and not loose and not links.get(d) and not (m.get("native") or {}).get("crate"):
            continue
        a.say(f"{where}")
        for n, x in sorted(decls[d].items()):
            if x.get("kind") == "vendored":
                check_vendored(tree, d, n, x, where, a, offline)
            elif x.get("kind") == "system":
                check_system(n, x, where, a, offline)
            elif x.get("kind") == "sdk":
                check_sdk(tree, n, x, where, a, offline)
        crate = (m.get("native") or {}).get("crate")
        if crate and not any(f.startswith((d + "/") if d else "") and f.endswith("Cargo.lock") for f in tree.files):
            a.need(where, f"Rust crate `{crate}` ships without a Cargo.lock, so its dependencies are unaudited")

    # Every lockfile shipped is audited, declared or not; a `cargo` entry carries its verdicts.
    locks = [f for f in tree.files if os.path.basename(f) == "Cargo.lock" and shipped(f)]
    for rel, (n, x, where) in sorted(cargo_decls.items()):
        if rel not in locks:
            a.need(where, f"{n}: the lockfile {rel} is not in the tree")
    for f in locks:
        od = owner_pkg(f, pkgdirs)
        n, x, where = cargo_decls.get(f) or (None, {}, wheres.get(od, tree.label))
        if n is None:
            a.need(where, f"{f}: Rust crates ship from this lockfile and no `cargo` entry declares it")
        named = {lib_base(y) for s_ in scope(od if od is not None else "").values() if s_.get("kind") == "system"
                 for y in split_list(s_.get("names"))}
        cargo_audit(tree.root, f, n or os.path.dirname(f) or "Cargo.lock", x, where, a, offline, named)


def advisories_for(x, where, n, a, offline):
    """Ask each register the declaration names; returns {id: (date, text)} or None when offline."""
    if offline:
        return None
    found = {}
    for reg in split_list(x.get("advisories")):
        kind, _, arg = reg.partition(":")
        try:
            if kind == "nvd":
                found.update(reg_nvd(arg))
            elif kind == "osv" and arg == "commit":
                if not x.get("commit"):
                    a.need(where, f"{n}: `osv:commit` needs a `commit`")
                    continue
                found.update(reg_osv_commit(x["commit"]))
            elif kind == "github":
                found.update(reg_github(arg))
            else:
                a.need(where, f"{n}: unknown advisory register `{reg}`")
        except (urllib.error.URLError, RuntimeError, OSError, ValueError) as e:
            a.need(where, f"{n}: the register `{reg}` could not be asked ({e}) — nothing is known either way")
    return found


def triage(n, x, where, found, a):
    if found is None:
        return "registers not asked"
    if not split_list(x.get("advisories")):
        a.need(where, f"{n}: no `advisories` register is named, so nothing can report against it")
        return "no register"
    reviewed = x.get("reviewed", {}) or {}
    open_ = applies = outside = 0
    for ident, (date, text, *rest) in sorted(found.items()):
        verdict = str(reviewed.get(ident, ""))
        ranges = rest[0] if rest else []
        if not verdict and ranges and x.get("version"):
            hits = [in_range(x["version"], r) for r in ranges]
            if all(h is False for h in hits):
                outside += 1
                continue
            text = f"[affects {' | '.join(ranges)}] {text}"
        word = verdict.split(":", 1)[0].strip()
        if not verdict:
            open_ += 1
            a.need(where, f"{n}: {ident} ({date}) is not reviewed — {text[:110].strip()}")
        elif word not in VERDICTS:
            a.need(where, f"{n}: {ident}: the verdict must start `fixed:`, `not-reached:` or `applies:`")
        elif word == "applies":
            applies += 1
            a.need(where, f"{n}: {ident} APPLIES to what ships — {verdict.split(':', 1)[1].strip()[:110]}")
    if not found:
        return "no advisories"
    s = f"{len(found)} advisories" + (f" ({outside} outside our version)" if outside else "")
    if open_ or applies:
        s += f" ({open_} unreviewed, {applies} apply)"
    else:
        s += ", all reviewed"
    return s


def check_vendored(tree, d, n, x, where, a, offline):
    rel = posix(os.path.join(d, x.get("path", "")))
    path = os.path.join(tree.root, rel)
    files = sorted(posix(os.path.relpath(os.path.join(dd, f), path))
                   for dd, _, fs in os.walk(path) for f in fs if f.endswith(CODE_EXT)) if os.path.isdir(path) else []
    if not files:
        a.need(where, f"{n}: `path` {rel} holds no code")
        return
    for key in ("upstream", "commit", "digest"):
        if not x.get(key):
            a.need(where, f"{n}: a `vendored` entry needs `{key}`"
                   + (f" ({tree_digest(path)})" if key == "digest" else ""))
    if x.get("digest") and x["digest"] != tree_digest(path):
        a.need(where, f"{n}: {rel} no longer matches its `digest` — something changed it since it was declared")
    status, fresh, tier = [], "", (0, "")
    commit = x.get("commit", "")
    if not offline and x.get("upstream") and commit:
        try:
            clone = upstream_clone(x["upstream"])
            by = upstream_blobs(clone, commit)
            renames = x.get("renames", {}) or {}
            allowed = set(split_list(x.get("modified")))
            theirs_names = set()
            for f in files:
                b = os.path.basename(f)
                t = renames.get(b, b)
                theirs_names.add(t)
                if git_blob_id(os.path.join(path, f)) not in by.get(t, {}) and b not in allowed:
                    a.need(where, f"{n}: {f} is not upstream's {t} at {commit[:9]} (list it in `modified` if that is meant)")
            # A fork is followed on its own branch (`branch = "stb_vorbis-sezero"`), not the
            # repository's default one.
            ref = f"refs/heads/{x['branch']}" if x.get("branch") else "HEAD"
            head = run(["git", "-C", clone, "rev-parse", ref], check=True).stdout.strip()
            behind = int(run(["git", "-C", clone, "rev-list", "--count", f"{commit}..{head}"]).stdout.strip() or 0)
            newest = next((t for t in run(["git", "-C", clone, "tag", "--sort=-v:refname"]).stdout.split()
                           if re.search(r"\d", t) and not re.search(r"(?i)rc|beta|alpha|dev|pre", t)), "")
            pin = run(["git", "-C", clone, "describe", "--tags", commit]).stdout.strip()
            since = x.get("upstream-reviewed") or commit
            changed = sorted({os.path.basename(p) for p in run(
                ["git", "-C", clone, "diff", "--name-only", since, head]).stdout.split()} & theirs_names)
            fresh = "upstream current" if behind == 0 else f"{behind} upstream commits since" + (
                f", newest tag {newest}" if newest and newest not in pin else "")
            if x.get("fork-of"):
                fresh = f"fork {x.get('branch') or 'HEAD'}: " + fresh
            ours_paths = sorted({p for t in theirs_names for p in by.get(t, {}).values()})
            subjects = run(["git", "-C", clone, "log", "--format=%s", f"{since}..{head}", "--", *ours_paths]
                           ).stdout.splitlines() if ours_paths else []
            kinds = classify(subjects)
            fork_note = check_fork(clone, head, ours_paths, n, x, where, a)
            counts = ", ".join(f"{len(v)} {k}" for k, v in kinds.items() if v)
            if changed:
                a.need(where, f"{n}: upstream changed {len(changed)} of our files since "
                       f"{since[:9]} ({', '.join(changed[:6])}{' …' if len(changed) > 6 else ''}; "
                       f"{counts or 'no commit subjects'}) — read the commits, then update or set "
                       f"`upstream-reviewed = \"{head}\"`")
            if kinds["security"]:
                tier = (1,
                        f"{len(kinds['security'])} security-class fix(es) to our files: " + "; ".join(kinds["security"][:3]))
            elif kinds["fix"]:
                tier = (2, f"{len(kinds['fix'])} fix(es) to our files: " + "; ".join(kinds["fix"][:3]))
            elif kinds["perf"] or kinds["other"]:
                tier = (3, f"{counts} to our files, no fix among them")
            elif behind:
                tier = (4, "upstream moved, none of it in our files")
            else:
                tier = (0, "")
            status.append(pin or commit[:9])
            if fork_note:
                fresh += " · " + fork_note
        except RuntimeError as e:
            a.need(where, f"{n}: the upstream {x['upstream']} could not be read ({e})")
    else:
        status.append(x.get("version") or commit[:9] or "?")
    found = advisories_for(x, where, n, a, offline)
    adv = triage(n, x, where, found, a)
    if applies_declared(x, found):
        tier = (1, "an advisory applies")
    if not offline:
        fresh = (fresh + " · " + set_tier(where, n, *tier)).strip(" ·")
    a.say(f"  {n:<16} vendored  {' '.join(status):<22} {fresh:<40} {adv}")


def check_fork(clone, head, ours_paths, n, x, where, a):
    """A fork we follow instead of its original (`fork-of`) is watched both ways: why we follow
    it is on record (`fork-reason`), and work the ORIGINAL does to our files that the fork does
    not carry needs action — the original may have woken up, or fixed something the fork lacks.
    The fork's own new commits are reviewed like any upstream change (the tiers above)."""
    orig = x.get("fork-of")
    if not orig:
        if x.get("branch"):
            a.need(where, f"{n}: follows branch `{x['branch']}`; say which project it forks (`fork-of`) "
                   "and why we follow it (`fork-reason`)")
        return ""
    if not x.get("fork-reason"):
        a.need(where, f"{n}: follows a fork of {orig}; say why in `fork-reason`")
    if OFFLINE or not ours_paths:
        return ""
    r = run(["git", "-C", clone, "fetch", "-q", orig, "+HEAD:refs/fork-of/head"])
    if r.returncode != 0:
        a.need(where, f"{n}: the original {orig} could not be read ({r.stderr.strip()[-120:]})")
        return ""
    since = x.get("fork-of-reviewed")
    rng = f"{since}..refs/fork-of/head" if since else f"{head}..refs/fork-of/head"
    missing = run(["git", "-C", clone, "log", "--format=%h %s", rng, "--", *ours_paths]).stdout.splitlines()
    if since:   # what was read up to `since` and is still not in the fork
        missing = [m for m in missing if run(["git", "-C", clone, "merge-base", "--is-ancestor",
                                              m.split()[0], head]).returncode != 0]
    own = run(["git", "-C", clone, "rev-list", "--count", f"refs/fork-of/head..{head}", "--", *ours_paths]).stdout.strip()
    if missing:
        tip = run(["git", "-C", clone, "rev-parse", "refs/fork-of/head"]).stdout.strip()
        a.need(where, f"{n}: the original {orig} has {len(missing)} commit(s) to our files the fork does not "
               f"carry ({'; '.join(m.split(' ', 1)[1] for m in missing[:3])}) — take them, ask the fork, or set "
               f"`fork-of-reviewed = \"{tip}\"`")
    return f"{own} fork commits of its own to our files"


def classify(subjects):
    c = {"security": [], "fix": [], "perf": [], "other": []}
    for sub in subjects:
        if TRIVIAL_MSG.search(sub.strip()):
            continue
        k = ("security" if SECURITY_MSG.search(sub) else "fix" if FIX_MSG.search(sub)
             else "perf" if PERF_MSG.search(sub) else "other")
        c[k].append(sub)
    return c


def applies_declared(x, found=None):
    """Does an advisory that `applies` still hold?  Counted over what the registers reported
    this run when they were asked, so a verdict about one lockfile's crate, or an advisory an
    update made go away, does not mark something else."""
    rv = x.get("reviewed") or {}
    ids = rv.keys() if found is None else [i for i in found if i in rv]
    return any(str(rv[i]).split(":", 1)[0].strip() == "applies" for i in ids)


def set_tier(where, n, tier, why):
    """The update policy's tier for one dependency: 1 now, 2 this release cycle, 3 only with a
    measured reason, 4 nothing of ours changed (THIRDPARTY.md § When to update)."""
    TIERS[f"{where}: {n}"] = (tier, why)
    return f"tier {tier}" if tier else "current"


def check_system(n, x, where, a, offline):
    names = split_list(x.get("names"))
    if not names:
        a.need(where, f"{n}: a `system` entry needs `names`")
    if not x.get("provider"):
        a.need(where, f"{n}: a `system` entry needs `provider` — who ships and patches it")
    adv = triage(n, x, where, advisories_for(x, where, n, a, offline), a) if x.get("advisories") else \
        f"patched by: {x.get('provider', '?')}"
    a.say(f"  {n:<16} system    {' '.join(names)[:62]:<63} {adv}")


def newest_versions(source):
    """(newest stable version overall, newest in our minor series helper list) from the source."""
    if source.startswith("esp-registry:"):
        d = json.loads(http("https://components.espressif.com/api/components/" + source.split(":", 1)[1]))
        return [v["version"] for v in d.get("versions", []) if not v.get("yanked_at")]
    out = run(["git", "ls-remote", "--tags", source]).stdout
    return [t for t in (re.sub(r"\^\{\}$", "", l.split("refs/tags/")[-1]) for l in out.splitlines())
            if re.fullmatch(r"v?\d+(\.\d+)*", t)]


def check_sdk(tree, n, x, where, a, offline):
    ver, src = str(x.get("version", "")), norm_source(x.get("source"))
    for key in ("source", "version", "pinned"):
        if not x.get(key):
            a.need(where, f"{n}: an `sdk` entry needs `{key}`")
    pin = str(x.get("pinned", ""))
    if ":" in pin:
        f, const = pin.split(":", 1)
        try:
            with open(os.path.join(tree.root, f), encoding="utf-8") as fh:
                m = re.search(rf'const {re.escape(const)}: &str = "((?:[^"\\]|\\.)*)"', fh.read())
        except OSError:
            m = None
        if not m:
            a.need(where, f"{n}: `pinned` names {pin}, which is not a `const {const}: &str` there")
        elif ver not in m.group(1):
            a.need(where, f"{n}: declared {ver}, but {pin} pins {m.group(1)!r} — update the declaration")
    fresh = ""
    if not offline and src:
        try:
            vs = sorted(set(newest_versions(src)), key=vkey)
            if vs:
                newest = vs[-1]
                series = [v for v in vs if vkey(v)[:2] == vkey(ver)[:2]]
                parts = []
                if series and vkey(series[-1]) > vkey(ver):
                    parts.append(f"{series[-1]} in our series")
                if vkey(newest) > vkey(ver) and (not series or newest != series[-1]):
                    parts.append(f"newest {newest}")
                fresh = ", ".join(parts) if parts else "current"
                if series and vkey(series[-1]) > vkey(ver):
                    a.need(where, f"{n}: {series[-1]} is out (bugfixes in our {'.'.join(map(str, vkey(ver)[:2]))} "
                           f"series); we pin {ver}")
        except (urllib.error.URLError, RuntimeError, OSError, ValueError) as e:
            a.need(where, f"{n}: the source {src} could not be asked for versions ({e})")
    found = advisories_for(x, where, n, a, offline)
    tier = (1, "an advisory applies") if applies_declared(x, found) else (0, "")
    if not tier[0] and "in our series" in fresh:
        tier = (2, f"a bugfix release in our series ({fresh})")
    elif not tier[0] and "newest" in fresh:
        tier = (3, f"a new series only ({fresh})")
    if fresh:
        fresh = fresh + " · " + set_tier(where, n, *tier)
    comps = []
    if not offline and src.startswith("esp-registry:"):
        try:
            comps = esp_components(src.split(":", 1)[1], ver.lstrip("="), x.get("target", "esp32p4"))
        except (urllib.error.URLError, RuntimeError, OSError, ValueError, KeyError) as e:
            a.need(where, f"{n}: its component dependencies could not be resolved ({e})")
        for cname, cver, repo, newest in comps:
            if repo and found is not None:
                try:
                    found.update({k: v for k, v in reg_github(repo).items()})
                except RuntimeError as e:
                    a.need(where, f"{n}: the advisories of {cname} ({repo}) could not be asked ({e})")
    adv = triage(n, x, where, found, a)
    a.say(f"  {n:<16} sdk       {ver:<22} {fresh:<40} {adv}")
    if comps:
        a.say(f"  {'':<16}   brings in {len(comps)} components: " + ", ".join(
            f"{c} {v}" + (f" (newest {nw})" if nw and vkey(nw) > vkey(v) else "") for c, v, _, nw in comps))


def esp_components(name, version, target):
    """[(component, version, github owner/repo or None, newest)] — what an ESP component pulls in,
    resolved through the component registry for `target`, the IDF itself left out."""
    def info(nm):
        return json.loads(http("https://components.espressif.com/api/components/" + nm))

    out, seen, todo = [], {name}, [(name, version)]
    while todo:
        nm, v = todo.pop(0)
        vs = info(nm)["versions"]
        entry = next((e for e in vs if e["version"] == v), None) or vs[0]
        for dep in entry.get("dependencies") or []:
            if dep.get("source") == "idf" or not dep.get("name"):
                continue
            rules = [r.get("if", "") for r in dep.get("rules") or []]
            if any(re.fullmatch(r"target\s*==\s*(\w+)", r) and re.fullmatch(r"target\s*==\s*(\w+)", r).group(1) != target
                   for r in rules):
                continue
            dn = f"{dep.get('namespace') or 'espressif'}/{dep['name']}"
            if dn in seen:
                continue
            seen.add(dn)
            dvs = [e for e in info(dn)["versions"] if not e.get("yanked_at")]
            spec = dep.get("spec", "")
            pick = next((e for e in dvs if spec.startswith("=") and e["version"] == spec[1:]), None) or next(
                (e for e in dvs if vkey(e["version"])[:1] == vkey(spec)[:1]), dvs[0] if dvs else None)
            if not pick:
                continue
            repo = re.search(r"github\.com/([\w.-]+/[\w.-]+?)(?:\.git)?(?:/|$)",
                             str(pick.get("repository") or "") + " " + str(pick.get("url") or ""))
            out.append((dn, pick["version"], repo.group(1) if repo else None, dvs[0]["version"] if dvs else ""))
            todo.append((dn, pick["version"]))
    return out


def cargo_native(toml, offline):
    """(crates whose build script compiles C, {library: crate} the crates link with #[link]) over the
    dependency graph resolved for every shipped platform.  `libc` is the platform C library."""
    ids, pk = set(), {}
    fetched = False
    for triple in SHIPPED_TARGETS:
        cmd = ["cargo", "metadata", "--format-version", "1", "--locked", "--manifest-path", toml,
               "--filter-platform", triple] + (["--offline"] if offline else [])
        r = run(cmd)
        if r.returncode != 0 and offline and not fetched and "--offline" in r.stderr:
            # The other platforms' crates were never downloaded here (a CI leg fetches its own).
            # Offline means no advisory registers, not no crate sources: fetch them, once.
            fetched = True
            if run(["cargo", "fetch", "--locked", "--manifest-path", toml]).returncode == 0:
                r = run(cmd)
        if r.returncode != 0:
            err = r.stderr.strip()
            raise RuntimeError("its Cargo.lock does not match its Cargo.toml (`cargo metadata --locked` "
                               "refuses), so what builds is not what was audited" if "lock file" in err
                               else (err.splitlines() or ["cargo metadata failed"])[-1])
        m = json.loads(r.stdout)
        pk.update({p["id"]: p for p in m["packages"]})
        # What ships is what the members reach through NORMAL dependencies; a build-dependency
        # (cc, find-msvc-tools) runs on the build machine and is not part of the artefact.
        nodes = {nd["id"]: nd for nd in m["resolve"]["nodes"]}
        todo, seen = list(m.get("workspace_members") or []), set()
        while todo:
            i = todo.pop()
            if i in seen:
                continue
            seen.add(i)
            ids.add(i)
            for dep in nodes.get(i, {}).get("deps", []):
                if any(k.get("kind") is None for k in dep.get("dep_kinds", [])):
                    todo.append(dep["pkg"])
    compiles, linked = [], {}
    for i in sorted(ids):
        p = pk[i]
        if p.get("source") is None or p["name"] == "libc":
            continue
        top = os.path.dirname(p["manifest_path"])
        b = os.path.join(top, "build.rs")
        if os.path.exists(b):
            with open(b, encoding="utf-8", errors="replace") as fh:
                if BUILD_C.search(fh.read()):
                    compiles.append(f"{p['name']} {p['version']}")
        for d, _, fs in os.walk(top):
            for fn in fs:
                if not fn.endswith(".rs") or OTHER_OS_FILE.search(os.path.relpath(os.path.join(d, fn), top)):
                    continue
                with open(os.path.join(d, fn), encoding="utf-8", errors="replace") as fh:
                    text = fh.read()
                for mm in RUST_LINK.finditer(text):
                    before = text[max(0, mm.start() - 400):mm.start()].splitlines()[-4:]
                    oses = set(re.findall(r'target_os\s*=\s*"([a-z]+)"', "\n".join(before)))
                    if oses and not oses & SHIPPED_OS and "apple" not in "\n".join(before):
                        continue
                    linked.setdefault(lib_base(mm.group(1)), f"{p['name']} {p['version']}")
    return compiles, linked


def cargo_native_check(toml, x, n, where, a, named):
    if not os.path.exists(toml):
        return ""
    try:
        compiles, linked = cargo_native(toml, OFFLINE)
    except (RuntimeError, ValueError, KeyError) as e:
        a.need(where, f"{n}: the crate graph could not be read ({e}) — what its crates link is unknown")
        return ""
    listed = set(split_list(x.get("compiles-c")))
    for c in compiles:
        if c.split()[0] not in listed:
            a.need(where, f"{n}: Rust crate {c} compiles C into what ships (its build.rs); "
                   "review that C, then list the crate in `compiles-c`")
    for lib, crate in sorted(linked.items()):
        if lib not in named:
            a.need(where, f"{n}: Rust crate {crate} links `{lib}`; no `system` entry names it")
    return f", C from {', '.join(c.split()[0] for c in compiles)}" if compiles else ""


def cargo_audit(root, rel, n, x, where, a, offline, named=frozenset()):
    """RustSec over one lockfile: a vulnerability, and an unmaintained, unsound or yanked crate,
    each needs a verdict in the `cargo` entry's `reviewed` table, exactly as an advisory does."""
    lock = os.path.join(root, rel)
    if GATE:   # the gate asks what is linked, not what the advisory database says
        d = {}
        adv, pending = "advisories not asked (gate)", ""
        toml = os.path.join(os.path.dirname(lock), "Cargo.toml")
        native = cargo_native_check(toml, x, n, where, a, named)
        a.say(f"  {n:<16} cargo     {rel:<63} {adv}{native}")
        return
    if not shutil.which("cargo-audit"):
        a.need(where, "cargo-audit is not installed (`cargo install cargo-audit --locked`)")
        return
    r = run(["cargo", "audit", "--json", "-f", lock] + (["--no-fetch"] if offline else []))
    try:
        d = json.loads(r.stdout)
    except json.JSONDecodeError:
        a.need(where, f"{n}: cargo audit could not answer: {(r.stderr.strip().splitlines() or ['?'])[-1]}")
        return
    found = {}
    for v in d.get("vulnerabilities", {}).get("list", []):
        adv, pkg = v["advisory"], v["package"]
        found[adv["id"]] = (adv.get("date", ""), f"{pkg['name']} {pkg['version']}: {adv['title']}")
    for kind, items in (d.get("warnings") or {}).items():
        for w in items:
            adv, pkg = w.get("advisory") or {}, w["package"]
            ident = adv.get("id") or f"{kind}:{pkg['name']}@{pkg['version']}"
            found[ident] = (adv.get("date", ""), f"{pkg['name']} {pkg['version']} is {kind}"
                            + (f": {adv['title']}" if adv.get("title") else ""))
    adv = triage(n, {**x, "advisories": ["cargo-audit"]}, where, found, a)
    pending = ""
    toml = os.path.join(os.path.dirname(lock), "Cargo.toml")
    if not offline and os.path.exists(toml):
        u = run(["cargo", "update", "--dry-run", "--manifest-path", toml])
        if u.returncode == 0:
            k = sum(1 for line in u.stderr.splitlines() if line.strip().startswith("Updating ") and " -> " in line)
            pending = f", {k} patch updates pending" if k else ", up to date"
    native = cargo_native_check(toml, x, n, where, a, named)
    count = d.get("lockfile", {}).get("dependency-count", "?")
    rv = x.get("reviewed") or {}
    bad = applies_declared(x, found) or any(k.startswith("yanked:") for k in found) or any(
        v["advisory"]["id"] not in rv for v in d.get("vulnerabilities", {}).get("list", []))
    k_pending = int(re.search(r"(\d+) patch", pending).group(1)) if re.search(r"(\d+) patch", pending) else 0
    t = (1, "a vulnerable, yanked or unmaintained-and-reached crate") if bad else \
        (2, f"{k_pending} patch updates in our series") if k_pending else (0, "")
    a.say(f"  {n:<16} cargo     {f'{count} crates in {rel}':<63} {adv}{pending}{native} · "
          + set_tier(where, f"{n} ({rel})", *t))


def main():
    global REFRESH, OFFLINE
    if len(sys.argv) > 1 and sys.argv[1] == "digest":
        print(tree_digest(sys.argv[2]))
        return 0
    if len(sys.argv) > 1 and sys.argv[1] == "identify":
        identify(sys.argv[2], sys.argv[3])
        return 0
    ap = argparse.ArgumentParser(description="Audit the third-party code loft and its libraries ship.")
    ap.add_argument("--check", action="store_true", help="print only what needs action")
    ap.add_argument("--no-libs", action="store_true", help="audit loft only")
    ap.add_argument("--repo", action="append", default=[], help="audit this local tree instead")
    ap.add_argument("--source", choices=("release", "main"), default="release",
                    help="libraries as released (default) or each repository's main")
    ap.add_argument("--offline", action="store_true", help="tree checks only")
    ap.add_argument("--gate", action="store_true",
                    help="the CI and publish gate: --offline, --check, and no advisory database — "
                         "fails on anything undeclared, never on what the registers say")
    ap.add_argument("--refresh", action="store_true", help="ignore the day-long cache")
    ap.add_argument("--release", action="store_true",
                    help="the release question: a tier 1 or 2 update not yet taken needs action too")
    ap.add_argument("--binary", help="the built loft to read what it links (default target/release/loft)")
    args = ap.parse_args()
    if args.gate:   # a gate reads a tree it was handed: loft's, or a library's via --repo
        args.offline = args.check = True
        args.no_libs = args.no_libs or not args.repo
    REFRESH, OFFLINE = args.refresh, args.offline
    global GATE, BINARY
    GATE, BINARY = args.gate, (os.path.abspath(args.binary) if args.binary else None)
    a = Audit()
    if args.repo:
        trees = [(os.path.basename(os.path.abspath(r)), lambda r=r: os.path.abspath(r)) for r in args.repo]
    else:
        sha = run(["git", "-C", ROOT, "rev-parse", "--short", "HEAD"]).stdout.strip()
        dirty = " + uncommitted" if run(["git", "-C", ROOT, "status", "--porcelain"]).stdout.strip() else ""
        trees = [(f"loft (this checkout, {sha}{dirty})", lambda: ROOT)]
        if not args.no_libs:
            try:
                trees += registry_trees(args.source)
            except (urllib.error.URLError, RuntimeError, OSError, ValueError) as e:
                a.need("libraries", f"the registry index could not be read ({e}) — no library was audited")
    for label, root in trees:
        try:
            r = root()
        except (urllib.error.URLError, RuntimeError, OSError, tarfile.TarError) as e:
            a.need(label, f"could not be fetched ({e})")
            continue
        t = Tree(label, r, is_loft=os.path.abspath(r) == ROOT)
        audit_tree(t, a, args.offline)
    if args.release:
        for k, (t, why) in sorted(TIERS.items()):
            if t in (1, 2):
                a.need(k.split(": ", 1)[0], f"{k.split(': ', 1)[1]}: a tier {t} update is not taken — {why[:140]}")
    if not args.check:
        print("\n".join(a.lines))
        if any(t for t, _ in TIERS.values()):
            print("\nUpdates by tier (THIRDPARTY.md § When to update):")
            for tier, label in ((1, "now"), (2, "this release cycle"), (3, "only with a measured reason"),
                                (4, "nothing of ours changed")):
                for k, (t, why) in sorted(TIERS.items()):
                    if t == tier:
                        print(f"  {tier} {label:<28} {k} — {why[:150]}")
    for note in dict.fromkeys(STALE_ANSWERS):
        print(f"note: {note}")
    if a.action:
        print(f"\n{len(a.action)} need action:" if not args.check else f"{len(a.action)} need action:")
        for where, what in a.action:
            print(f"  {where}: {what}")
        return 1
    if not args.check:
        print("\nNothing needs action.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
