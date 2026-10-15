// The compiled standard library with nothing in it — what a falsification control starts from.
//
// `scripts/falsify.sh --patch` never applies a receipt's hunk of `src/compiled_stdlib_gen.rs`:
// that file is generated from the tree it sits in, so the control puts this stub in its place,
// builds, and has its own generator write the real one (GUARDS.md § The patch receipt).  With
// no bridges `compiled_stdlib::mark` declines, so the stub build interprets every standard
// library body — enough to run the generator, never what a control is scored on.
//
// It declares exactly what `src/compiled_stdlib.rs` reads from the generated module;
// `tests/compiled_stdlib.rs` fails when the two drift apart.

pub(crate) static BRIDGES: &[(&str, &str, super::Bridge)] = &[];
pub(crate) const PREFIX_TYPES: usize = 0;
pub(crate) const PREFIX_FINGERPRINT: u64 = 0;
pub(crate) const SOURCE_HASH: u64 = 0;
