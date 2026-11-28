# Agent Instructions for Loft

Read [CLAUDE.md](CLAUDE.md) in full before working in this repository. It contains the mandatory project rules, architecture overview, build/test commands, environment switches, and links to all subsystem docs in `doc/claude/`.

## Mandatory Highlights (see `CLAUDE.md` for full rules)

- **Branch & Git Safety:** Never commit on `main`. Never create a branch or open a PR without an explicit user ask. Never run `git bisect`, `git stash pop`, `git pull`, or `git checkout -- <file>` with uncommitted changes.
- **Skills:** Check `.agents/skills/` (symlinked to `.claude/skills/`) and load the matching `SKILL.md` before debugging (`loft-debug`, `engineering-rigor`), testing (`loft-test`), writing `.loft` (`loft-write`), codegen (`loft-codegen`), optimization (`loft-optimize`), libraries (`loft-library`), design (`design-protocol`, `formal-rules`), docs (`doc-quality`), or file splitting (`split-file`).
- **Verification:** Verify fixes on both the interpreter and the C backend (`--native-c`, `make suite`, `scripts/find_problems.sh --changed`).
