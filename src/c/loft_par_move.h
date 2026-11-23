/* loft_par_move.h — a threaded `par` site's adopt step for a record result (STORE_MODEL D46),
 * after the keyed collections (`loft_keyed.h`: the par buffer and `loft_par_run`) and the image
 * walk (`loft_image.h`: `loft_kind_slots`).  The emitted adopt step of each record site calls it
 * (`par_move_fn`); the runtime itself does not, so a unit of the runtime alone, the self-host
 * shim, leaves it out. */

/* STORE_MODEL D46 (`Stores::adopt_worker_excess` + `rebase_walk_record`): the rows `lo..hi`
 * whose record lies in a store the worker made (number `parent_n` or past it) take that store
 * into the caller's table, in its lowest free slot and uncounted, as the interpreter's
 * `adopt_store`.  The row names the new slot and is ADOPTED, so the buffer's drop frees it; rows
 * naming one store share it, the first adopting it.  Then every stored reference the rows' records
 * reach (type `kt`) that names a worker's store is renumbered, the store it names taken in too
 * when no row brought it (`loft_par_hold`), and followed into its target.  A reference to a
 * store the caller had stays as it is. */
typedef struct { loft_store_t *wt; uint32_t wn, parent_n; uint16_t *moved; } loft_par_move;

/* Worker store `k` (`k >= parent_n`) in the caller's table: its slot there, taken now if no
 * row or reference brought it yet; `fresh` says it was. */
static uint16_t loft_par_take(loft_cx *cx, loft_par_move *m, uint32_t k, int *fresh) {
    uint16_t *slot = &m->moved[k - m->parent_n];
    *fresh = *slot == 0xFFFF;
    if (*fresh) {
        uint32_t nr = loft_store_make(cx, 0);
        loft_store_t *s = &cx->stores[nr];
        loft_heap_moved(cx, s, s->size, 0);
        free(s->buf);
        free(s->claims.bits);
        free(s->referenced.bits);
        free(s->removed.bits);
        free(s->lazy_free.bits);
        free(s->parked_set.bits);
        free(s->placed);
        *s = m->wt[k];
        memset(&m->wt[k], 0, sizeof m->wt[k]);
        *slot = (uint16_t) nr;
    }
    return *slot;
}

static void loft_par_move_rows(loft_cx *cx, const loft_tdesc *T, uint32_t ntypes, uint16_t kt, loft_par_slot *b, loft_word lo,
                               loft_word hi, uint32_t parent_n) {
    loft_par_move m = { cx->stores, cx->nstores, parent_n, 0 };
    m.moved = (uint16_t *) malloc((size_t) (m.wn > parent_n ? m.wn - parent_n : 1) * sizeof *m.moved);
    if (!m.moved) { fputs("loft: out of memory\n", stderr); abort(); }
    for (uint32_t k = parent_n; k < m.wn; k++) m.moved[k - parent_n] = 0xFFFF;
    cx->stores = cx->par_caller;
    cx->nstores = cx->par_caller_n;
    cx->hint_table = 0;
    /* The records to walk: each row's, then each reference's target; `seen` keeps a record
     * walked once, so a cycle of references ends. */
    loft_refs todo = { 0, 0, 0, 0 };
    todo.tp = (uint16_t *) malloc(16 * sizeof *todo.tp);
    if (!todo.tp) { fputs("loft: out of memory\n", stderr); abort(); }
    for (loft_word i = lo; i < hi; i++) {
        loft_ref r = b[i].r;
        if (r.store_nr == 0xFFFF || r.rec == 0 || r.store_nr < parent_n || r.store_nr >= m.wn
            || (m.moved[r.store_nr - parent_n] == 0xFFFF && !m.wt[r.store_nr].live)) {
            b[i].adopted = 0;
            continue;
        }
        int fresh;
        b[i].r.store_nr = loft_par_take(cx, &m, r.store_nr, &fresh);
        b[i].adopted = (uint8_t) fresh;
        loft_refs_push_tp(cx, &todo, b[i].r, kt);
    }
    loft_refs seen = { 0, 0, 0, 0 };
    for (size_t w = 0; w < todo.n; w++) {
        loft_ref at = todo.p[w];
        uint16_t tp = todo.tp[w];
        int again = 0;
        for (size_t v = 0; v < seen.n && !again; v++)
            again = seen.p[v].store_nr == at.store_nr && seen.p[v].rec == at.rec && seen.p[v].pos == at.pos;
        if (again || tp >= ntypes) continue;
        loft_refs_push(cx, &seen, at);
        loft_refs slots = { 0, 0, 0, 0 };
        slots.tp = (uint16_t *) malloc(16 * sizeof *slots.tp);
        if (!slots.tp) { fputs("loft: out of memory\n", stderr); abort(); }
        loft_kind_slots(cx, T, tp, at, &slots, 0, LOFT_T_DBREF);
        for (size_t k = 0; k < slots.n; k++) {
            /* The stored words as they stand: the number names a store of the worker's table,
             * which no read through the caller's may resolve. */
            loft_ref sl = slots.p[k];
            loft_store_t *hs = loft_store_of(cx, sl);
            loft_ref x = { (uint16_t) loft_u32(cx, hs, sl.rec, sl.pos), loft_u32(cx, hs, sl.rec, sl.pos + 4),
                           loft_u32(cx, hs, sl.rec, sl.pos + 8), 0 };
            /* A reference into a store the worker freed names nothing to take: the interpreter
             * adopts only live stores (`adopt_worker_excess`), and leaves such a number as it is. */
            if (x.store_nr == 0xFFFF || x.store_nr < parent_n || x.store_nr >= m.wn) continue;
            if (m.moved[x.store_nr - parent_n] == 0xFFFF && !m.wt[x.store_nr].live) continue;
            int fresh;
            x.store_nr = loft_par_take(cx, &m, x.store_nr, &fresh);
            if (fresh) loft_par_hold(cx, x.store_nr);
            loft_set_u32(cx, loft_store_of(cx, sl), sl.rec, sl.pos, x.store_nr);
            if (slots.tp[k] != 0xFFFF && x.rec != 0) loft_refs_push_tp(cx, &todo, x, slots.tp[k]);
        }
        free(slots.p);
        free(slots.tp);
    }
    free(todo.p);
    free(todo.tp);
    free(seen.p);
    free(m.moved);
    cx->par_caller = cx->stores;
    cx->par_caller_n = cx->nstores;
    cx->stores = m.wt;
    cx->nstores = m.wn;
    cx->hint_table = 0;
}
