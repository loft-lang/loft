# Changing a library other libraries build on

What to do when a change to one library reaches the libraries that depend on it: which
floors move, in which order the releases go out, and which checks tell you. The contract
for one library on its own is [LIBRARY_AUTHORING.md § 3](LIBRARY_AUTHORING.md); the three
declared levels are in [COMPATIBILITY.md](COMPATIBILITY.md).

## When a change reaches the dependents

A dependent is reached when the change breaks the base library's own claim. Ask the base
first, with every earlier release installed (§ The checks, and when they say nothing):

```bash
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

## The checks, and when they say nothing

| Check | Run by | Says nothing when |
|---|---|---|
| `loft compat check` | the library CI, on every PR | no earlier release of the package is installed: it prints `no other release installed` and passes (loft#2599) <!-- doc-lint: ok --> |
| `loft compat check --full` | the publish script, before the tag | the same: it verifies the releases the machine has, not the ones the registry lists |
| `python3 scripts/thirdparty.py --gate --repo <pkg>` | the library CI and the publish script | never, but it reads every C file of the package, tests' helpers included |

So before a change to a base library, and before publishing one, install what the check
is to compare against, and read its first line for how many releases it looked at:

```bash
loft install <base>@<each published version>
cd <base> && loft compat check --full      # "verifying all N installed release(s)": N is the claim
```

A layout break is held against `api_compatible_with`, not `data_compatible_with`
(loft#2598): a dependent of step 3 whose API did not change raises both floors to pass. <!-- doc-lint: ok -->
Write beside the API floor that the API is unchanged, so the next reader does not look for
the break.

The publishing checkout must be at a `main` that has every header and rule the libraries
use: a refusal that names a file of loft's own (`loft_width.h`) is a stale checkout, not a
defect in the library.
