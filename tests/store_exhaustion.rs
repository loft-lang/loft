// #306 regression — the store-slot space is u16 and store_nr 65535 is the
// null-DbRef sentinel.  Before the fix, allocating the 65536th live store
// wrapped the `max` watermark to 0 and the allocator handed out slot 0 (the
// interpreter's eval-stack store) as a fresh store, silently corrupting the
// whole runtime.  The allocator must now refuse loudly instead.
//
// #1851 — and say WHICH of two things filled the table.  With reuse off
// (`LOFT_STRICT_STORES`, `LOFT_NO_SLOT_REUSE`, a `par` worker) every store ever
// made holds a number, freed or not, so a loop that frees and re-mints fills it
// with almost nothing live — not a leak, and the stop must not call it one.
use loft::database::Stores;

// @speed 1.2
#[test]
#[should_panic(expected = "store table exhausted: 65535 stores live at once")]
fn slot_exhaustion_is_loud() {
    let mut stores = Stores::new();
    for _ in 0..66_000u32 {
        let _ = stores.database(16);
    }
}

/// Reuse off, every store freed at once: the table still fills, and the stop names the
/// checking mode and the true live count (none of the minted stores).
// @speed 1.2
#[test]
#[should_panic(expected = "store table exhausted by the checking mode, not by a leak")]
fn exhaustion_without_reuse_names_the_mode() {
    let mut stores = Stores::new();
    stores.disable_slot_reuse = true;
    for _ in 0..66_000u32 {
        let db = stores.database(16);
        stores.free(&db);
    }
}
