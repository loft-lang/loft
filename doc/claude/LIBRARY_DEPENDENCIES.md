# Changing a library other libraries build on

What to do when a change to one library reaches the libraries that depend on it: which
floors move, in which order the releases go out, and which checks tell you. The contract
for one library on its own is [LIBRARY_AUTHORING.md § 3](LIBRARY_AUTHORING.md); the three
declared levels are in [COMPATIBILITY.md](COMPATIBILITY.md).

## When a change reaches the dependents

A dependent is reached when the change breaks the base library's own claim. Ask the base
first:

```bash
loft install <base>@<its last release>               # `compat api` reads the install cache
cd <base> && loft compat api <its last release>     # names each symbol and each reshaped type
```

| The base's verdict | Reaches |
|---|---|
| `API: drop-in`, `Layout: unchanged` | nobody: a patch release of the base |
| `API: BREAK` (a removed or changed name; **a field added to a public record counts**) | every package that depends on it |
| `Layout: CHANGED` | every package that depends on it, and again every package whose own records or signatures NAME a reshaped type |

A field added to a public record is both rows at once. It reads as an addition, and it is
the most common way to break a base library without meaning to. Before adding one, ask
whether the value can live somewhere that is not a public record.

## What moves, package by package

1. **The base** raises `api_compatible_with` and `data_compatible_with` to the release that
   carries the break. That is the declaration; the version number says nothing.
2. **Every dependent** raises its requirement on the base to that release
   (`gpu = ">=0.4.1"`) and takes a patch release. A dependent that still asks for the older
   line is HELD on it by the resolver: it keeps working, and it never receives the change.
3. **A dependent whose surface names a reshaped type** (it answers the base's record, takes
   one, or holds one in a public record of its own) has changed layout too. It raises its
   own `data_compatible_with` to its new release.
4. **Unpublished releases on the same branch** take the new requirement before they go out,
   so they are not released twice.

Find the dependents from the manifests, not from memory:

```bash
grep -l '^<base> *=' */loft.toml          # in the base's repository
make libcatalogue                         # then LIBRARIES.md, for the other repositories
```

## The order of release

One package at a time, stopping at the first refusal:

1. the base;
2. `loft install <base>@<version> --refresh` (the index is cached; without it the next
   publish resolves the old base);
3. each dependent, a dependent of a dependent after what it depends on;
4. the applications, last, from their own repositories.

Between 1 and 3 every dependent still resolves to the older base and still works: a
declared break leaves no window in which something is broken, only one in which the
change has not arrived.

## The checks, and what each one compares

| Check | Run by | Compares against |
|---|---|---|
| `loft compat check` | the library CI, on every PR | a sample of the releases the registry lists: the newest, the API floor, one drawn at random |
| `loft compat check --full` | the publish script, before the tag | every listed release at or above the lower of the two floors |
| `python3 scripts/thirdparty.py --gate --repo <pkg>` | the library CI and the publish script | every C file of the package, the helpers of its tests included |

A release the registry lists is fetched when it is not in the install cache, so the check
means the same on a machine that never installed the package. It fails, and says what it
could not verify, when the registry cannot be read and nothing is cached, and when a
release it still promises something about cannot be fetched. Its first line counts the
releases it looked at: read it.

Each kind of break is held against its own floor: a changed surface against
`api_compatible_with`, a reshaped record against `data_compatible_with`. The failure names
the floor to raise. A dependent of step 3 whose API did not change raises its data floor
alone, and its consumers are not held back.

The publishing checkout must be at a `main` that has every header and rule the libraries
use: a refusal that names a file of loft's own (`loft_width.h`) is a stale checkout, not a
defect in the library.
