<!-- reference for the loft-write skill: read before writing `use` of a library -->
# loft-write: finding a library's API

Split out of [SKILL.md](SKILL.md) (unchanged).

## Finding a library's API

Libraries live OUTSIDE the project (`~/.loft/registry/<name>-<version>/`,
`~/.loft/lib/<name>/`), so the project tree alone does not show what they
export.  Which surface is the truth depends on what you are doing:

- **Writing a program that USES a library** — the version your project locks is the
  one that runs, so its stubs are the truth.  Use the list below.
- **Working ON a library, inside the loft repo** — read `origin/main` through the
  catalogue: `make libcatalogue`, then `doc/claude/LIBRARIES.md` (CLAUDE.md, @PLN112).
  An installed copy or a clone can lag `origin/main`, and `loft api <name>` reads the
  installed copy.

Discovery surface for a program, nearest first:

1. **`.loft/api/<name>.api`** in the project — generated public-API stubs
   (signatures + doc comments) for every locked dependency.  Written by
   `loft install` / `loft update` / `loft pin`; read these first.
2. **`.loft/api/_available.api`** — the registry CATALOG: every package you
   could `loft install` (name, latest version, one-line description), written
   alongside the per-dep stubs.  Read this to see what EXISTS, not just what's
   installed.
3. **`loft api`** — list every library reachable from the cwd (project deps,
   installed registry packages, user libraries) with their source paths.
4. **`loft api <name>`** — print one library's full public surface.
5. **`loft api --registry`** — print the whole installable catalog on demand
   (the live form of `_available.api`).  The catalog is cached ~1h; add
   **`--refresh`** to force a re-fetch (e.g. after a package was just published
   or its description changed).
6. **`loft search <query>`** / **`loft info <name>`** — query the registry
   for libraries not installed yet; `loft install <name>` fetches one and
   refreshes the stubs.

Never guess a library function's signature: check the stub or `loft api`
output, and read the real source at the path they name when you need the
implementation.
