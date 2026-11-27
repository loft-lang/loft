# @PLN91 — registering generic type instances (tester, diff-noise work)

**Why:** generic type instances are the largest pool of definitions only Rust registers in
refused programs: 109 `Name<…>` instances, 69 instance variants (`Leaf#2`) and 116 vectors of
instances, in about 31 programs (try 1edef3fdb).  The port mints no instance at all: a type
`Box<integer>` is refused (`a type with arguments, keys, a limit or a member`, el_types.loft
`type_of`), and an enum template is refused before its header is bound, so a variant field
typed `T` mints a stub `T` that Rust never has (the port-only `T` rows).

**The invariant (register, then refuse):** wherever Rust's `Data::instance_def` mints an instance,
the port mints the same definitions: same name, kind, source, place in the numbering, and the
same fields.  It still refuses whatever it cannot lower, so no verdict moves; only the
definitions one side registers and the numbers after them change.

## The Rust routines and their port counterparts

| Rust (@PLN165) | what it decides | port |
|---|---|---|
| `bind_type_header` (definitions.rs) | a template's `type_params`, the header holders in header order | `parse_struct` / `parse_enum` template branches, el_definitions.loft |
| `parse_enum`'s `D-Template` arm | an enum with a header is a `TypeTemplate`, its variants read over the placeholders | `parse_enum`: bind the header, mark `DT_TYPE_TEMPLATE`, then refuse as now |
| `Data::instance_def` | `<identity><args>` in the standard library's source, idempotent; deferred while an argument is unresolved or `never`; an argument naming a type variable mints an OPEN instance; nesting bound 48 | el_instances.loft `instance_def` |
| `fill_enum_instance` | the instance's variant list and one `EnumValue` per template variant, bare-named, payload fields closed | el_instances.loft |
| `copy_instance_field` + `close_open` | each template field with the bindings applied, every open instance in it closed, tuple records first (innermost first), the field's flags | el_instances.loft |
| `parse_type_inner`'s `D-Template` arm | the type position: arguments read, arity checked, the instance minted | el_types.loft `type_of`: mint, then refuse |

Not in this slice: the struct-literal inference site (`objects.rs ~6659`), forward generic
returns (`resolve_forward_generic_returns`), `refresh_instances` / `template_is_regular`
(`@FR-G-Regular`, the catch-up of an instance minted while its own template is read), and
generic FUNCTION instances (`i_…`, 146 definitions).

## Validated by

A try with 0 verdict changes, the Rust-only count down by the instance rows, and no port-only
row new.  The catch-up gap shows as instances with fewer fields than Rust's (state and image
differences in still-refused programs), which ranks the next slice.
