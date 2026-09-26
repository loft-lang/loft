// @I116 — Library placement — in-process, a worker, or another machine

//! The Linux transport for @PLN119 placement: the shared mapping, the
//! spin-then-sleep handshake, the frame codec, and the two ends of a call.
//!
//! See the parent module for why the handshake spins before it sleeps and what
//! arc A does and does not carry across the boundary.

use super::arena::Arena;
use std::io;
use std::path::{Path, PathBuf};

/// The two arena files that sit beside a wire, derived from its path rather than
/// passed separately: three paths that must agree are three chances to disagree.
fn arena_paths(wire: &Path) -> (PathBuf, PathBuf) {
    let mut arg = wire.as_os_str().to_os_string();
    arg.push(".arg");
    let mut ret = wire.as_os_str().to_os_string();
    ret.push(".ret");
    (PathBuf::from(arg), PathBuf::from(ret))
}

/// `"LOFW"` — the first word of the mapping, so a stale or foreign file is
/// rejected rather than read as a call frame.
const MAGIC: u32 = 0x4C4F_4657;

/// Bumped whenever the frame encoding below changes. Caller and worker are
/// normally the same binary, but a stale worker executable is exactly the case
/// this catches.
const PROTOCOL: u32 = 2;

/// Total mapping size. Arc A's frames are scalars and text, so this is
/// generous; arc B sizes it from the argument graph instead.
const WIRE_BYTES: usize = 1 << 20;

// Header offsets. The two sequence words sit on separate 64-byte lines: they
// are written by opposite sides on every call, and sharing a line would trade
// the syscall we just removed for cache-line ping-pong.
const OFF_MAGIC: usize = 0;
const OFF_PROTOCOL: usize = 4;
const OFF_EPOCH: usize = 8;
const OFF_STORE_BASE: usize = 12;
const OFF_REQ_SEQ: usize = 64;
const OFF_REQ_SLEEPERS: usize = 68;
const OFF_RESP_SEQ: usize = 128;
const OFF_RESP_SLEEPERS: usize = 132;
const OFF_PAYLOAD: usize = 192;

/// How many times a side re-reads the sequence word before it sleeps.
///
/// Chosen from the Q4 measurement: a served call turns round in ~130 ns, so a
/// budget this size covers a worker that is answering promptly and expires
/// quickly on one that is not.
const SPIN_BUDGET: u32 = 2000;

/// How long a caller sleeps before looking to see the worker is still there.
///
/// Only a call that has already spun past its budget ever sleeps, so this is not
/// on the path of a busy exchange; what it bounds is how long a caller waits on
/// a worker that will never answer. Short enough that a death is reported
/// promptly, long enough that a genuinely slow library call costs a handful of
/// wakeups rather than a spin.
const LIVENESS_POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// A request kind, the first word of every request frame.
const REQ_CALL: u32 = 0;
const REQ_SHUTDOWN: u32 = 1;
/// Ask the worker how IT lays out a named function's compound types (@PLN119
/// arc B's layout gate). Answered once per function at install, never on a call.
const REQ_LAYOUT: u32 = 2;

/// Response status, the first word of every response frame.
const RESP_OK: u32 = 0;
const RESP_ERR: u32 = 1;

/// Value tags on the wire. These mirror [`crate::host::Value`] rather than
/// loft's `Type`, because the host marshaller is what both ends use.
const TAG_VOID: u32 = 0;
const TAG_BOOL: u32 = 1;
const TAG_INT: u32 = 2;
const TAG_FLOAT: u32 = 3;
const TAG_TEXT: u32 = 4;
/// A struct / vector: the `(rec, pos)` of its record in the call arena.
///
/// The store NUMBER is deliberately not on the wire. Each side registers the
/// arena at whatever slot its own `Stores` had free, and nothing inside a record
/// graph names a store (interior pointers are plain `u32` record ids), so the
/// receiver fills in its own — see [`super::arena`].
const TAG_REF: u32 = 5;
/// An absent struct / vector (`DbRef::NULL`).  A separate tag rather than a
/// reserved `rec`: record 0 is a real address, and an absent value that decoded
/// to one would read the arena's own header as a struct.
const TAG_NULLREF: u32 = 6;

/// What a failed crossing carries back.
///
/// The module's invariant names ERROR BEHAVIOUR beside type, effect and
/// lifetime, and a `String` channel cannot hold it: a fault inside a library has
/// a POSITION, a source line, a caret and the frames it fired under, and every
/// one of those was lost turning the fault into prose.  The placed rendering
/// then read `panic: runtime error: panic: refusing -1` — the message doubled
/// and nothing else — where in-process the same fault named the library's file,
/// line and call chain.
///
/// A TRANSPORT failure has none of those parts and does not pretend to: a dead
/// worker, an oversized frame or a malformed response converts from its own
/// message and leaves the rest empty.  That is what the `From` impls are for,
/// and why every `?` on this path still reads as it did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fault {
    /// The rendered detail line, exactly as the far side rendered it.  Never
    /// re-described on arrival: describing it again is what doubled `panic:`.
    pub message: String,
    /// The far side's kind label (`user_panic`, `divide_by_zero`, …), so a
    /// relayed fault stays the kind it was in a log.  Empty for a transport
    /// failure, which has no loft kind at all.
    pub label: String,
    /// Where in the LIBRARY's source it fired.
    pub position: Option<crate::lexer::Position>,
    /// The frames the library was in, innermost first.  The caller's own frames
    /// are added when the two meet — see [`crate::runtime_error::RuntimeError`]'s
    /// `crossed_placement`.
    pub call_chain: Vec<String>,
}

impl From<String> for Fault {
    fn from(message: String) -> Self {
        Fault {
            message,
            ..Fault::default()
        }
    }
}

impl From<&str> for Fault {
    fn from(message: &str) -> Self {
        Fault::from(message.to_string())
    }
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl Fault {
    /// The fault as the library's own side saw it.
    ///
    /// A runtime fault is relayed whole; anything else the host call can answer
    /// — an unknown function, an argument the signature does not admit — is a
    /// failure of the CROSSING rather than of the library's code, and has no
    /// loft position to carry.
    pub(crate) fn from_loft_error(e: &crate::host::LoftError) -> Self {
        match e {
            crate::host::LoftError::Runtime(err) => Fault {
                message: err.message.clone(),
                label: err.kind.label().to_string(),
                position: err.position.clone(),
                call_chain: err.call_chain.clone(),
            },
            other => Fault::from(other.to_string()),
        }
    }

    /// This fault with `extra` appended to its detail, keeping every other part.
    pub(crate) fn with_detail(mut self, extra: &str) -> Self {
        self.message.push_str(extra);
        self
    }
}

/// The shared mapping both processes address the call through.
///
/// Not a [`crate::store::Store`]: arc A carries scalars, so it needs a frame
/// buffer rather than a heap. Arc B replaces the payload region with a real
/// mmap-backed store so a value graph can be built in place, which is why the
/// header already reserves the epoch and store-base words.
pub struct Wire {
    base: *mut u8,
    /// Kept so the file outlives the mapping and unlinking is the owner's call.
    path: PathBuf,
    /// Only the process that created the file removes it.
    owner: bool,
}

// The mapping is shared by definition; what makes it safe is the sequence
// protocol, which hands ownership of the payload region to exactly one side at
// a time.
unsafe impl Send for Wire {}

impl Drop for Wire {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.base.cast::<libc::c_void>(), WIRE_BYTES);
        }
        if self.owner {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl Wire {
    /// Create the backing file and map it. Used by the caller, which owns the
    /// file's lifetime.
    ///
    /// # Errors
    /// Any failure to create, size, or map the file.
    pub fn create(path: &Path) -> io::Result<Wire> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len(WIRE_BYTES as u64)?;
        let w = Wire::map(&file, path, true)?;
        unsafe {
            w.base.write_bytes(0, WIRE_BYTES);
        }
        w.put_u32(OFF_PROTOCOL, PROTOCOL);
        // Magic last: it is what the worker waits on, so publishing it before
        // the rest is initialised would let the worker read a half-built header.
        w.publish_magic();
        Ok(w)
    }

    /// Map a file another process created. Used by the worker.
    ///
    /// # Errors
    /// A missing or unmappable file, a wrong magic (not our file), or a
    /// protocol mismatch (a stale worker executable against a newer caller).
    pub fn attach(path: &Path) -> io::Result<Wire> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        let w = Wire::map(&file, path, false)?;
        if w.get_u32(OFF_MAGIC) != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not a loft placement wire", path.display()),
            ));
        }
        let proto = w.get_u32(OFF_PROTOCOL);
        if proto != PROTOCOL {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "placement wire protocol {proto} but this worker speaks {PROTOCOL} — \
                     the worker executable is a different build from the caller"
                ),
            ));
        }
        Ok(w)
    }

    fn map(file: &std::fs::File, path: &Path, owner: bool) -> io::Result<Wire> {
        use std::os::fd::AsRawFd;
        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                WIRE_BYTES,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if base == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Wire {
            base: base.cast::<u8>(),
            path: path.to_path_buf(),
            owner,
        })
    }

    fn publish_magic(&self) {
        self.atomic(OFF_MAGIC)
            .store(MAGIC, std::sync::atomic::Ordering::SeqCst);
    }

    /// `mmap` returns page-aligned memory and every header offset above is a
    /// multiple of 4, so the `AtomicU32` alignment this cast needs is a
    /// property of the constants rather than a hope.
    #[allow(clippy::cast_ptr_alignment)]
    fn atomic(&self, off: usize) -> &std::sync::atomic::AtomicU32 {
        debug_assert_eq!(off % 4, 0, "header offset {off} is not u32-aligned");
        unsafe { &*self.base.add(off).cast::<std::sync::atomic::AtomicU32>() }
    }

    fn get_u32(&self, off: usize) -> u32 {
        self.atomic(off).load(std::sync::atomic::Ordering::SeqCst)
    }

    fn put_u32(&self, off: usize, v: u32) {
        self.atomic(off)
            .store(v, std::sync::atomic::Ordering::SeqCst);
    }

    /// The caller's store-numbering base, handed to the worker at attach so a
    /// `DbRef` it mints can be translated into the caller's numbering.
    ///
    /// Arc A never sends a reference, so nothing reads this yet. It is written
    /// at handshake anyway because the alternative — adding it when arc B needs
    /// it — is a protocol change that would silently mismatch a running worker.
    pub fn set_store_base(&self, base: u32) {
        self.put_u32(OFF_STORE_BASE, base);
    }

    /// See [`Wire::set_store_base`].
    #[must_use]
    pub fn store_base(&self) -> u32 {
        self.get_u32(OFF_STORE_BASE)
    }

    /// Stamp the caller's store generation, so a worker holding a mapping from
    /// before a structural change can tell that it is stale.
    pub fn set_epoch(&self, epoch: u32) {
        self.put_u32(OFF_EPOCH, epoch);
    }

    /// See [`Wire::set_epoch`].
    #[must_use]
    pub fn epoch(&self) -> u32 {
        self.get_u32(OFF_EPOCH)
    }

    fn payload(&self) -> *mut u8 {
        unsafe { self.base.add(OFF_PAYLOAD) }
    }

    // ── the two directions ──────────────────────────────────────────────

    /// Publish `seq` on a channel and wake the other side if it is asleep.
    fn publish(&self, seq_off: usize, sleepers_off: usize, seq: u32) {
        self.atomic(seq_off)
            .store(seq, std::sync::atomic::Ordering::SeqCst);
        // The SeqCst store above pairs with the waiter's SeqCst re-read after it
        // announces itself: whichever order the two run in, one of them observes
        // the other, so a wakeup cannot be lost.
        if self.get_u32(sleepers_off) != 0 {
            futex_wake(self.atomic(seq_off));
        }
    }

    /// Block until the channel's sequence passes `last`, spinning first.
    ///
    /// `poll` bounds each individual sleep, and `keep_waiting` is asked after
    /// every sleep that expires; answering `false` abandons the wait and returns
    /// `None`. Pass `None` to sleep until woken, which is what a side with an
    /// independent way of noticing the other has gone should do.
    ///
    /// The polling exists because a `FUTEX_WAIT` on a word nobody will ever
    /// write again does not fail — it waits forever. Only a wait that has
    /// already spun past its budget ever reaches a sleep, so a busy exchange
    /// never pays for this.
    fn await_past<F: FnMut() -> bool>(
        &self,
        seq_off: usize,
        sleepers_off: usize,
        last: u32,
        poll: Option<std::time::Duration>,
        mut keep_waiting: F,
    ) -> Option<u32> {
        let seq = self.atomic(seq_off);
        let sleepers = self.atomic(sleepers_off);
        let mut spun = 0u32;
        loop {
            let cur = seq.load(std::sync::atomic::Ordering::SeqCst);
            if cur != last {
                return Some(cur);
            }
            if spun < SPIN_BUDGET {
                spun += 1;
                std::hint::spin_loop();
                continue;
            }
            sleepers.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
            let cur = seq.load(std::sync::atomic::Ordering::SeqCst);
            if cur != last {
                sleepers.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                return Some(cur);
            }
            futex_wait(seq, cur, poll);
            sleepers.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            // Re-read before consulting the check: a wake and a timeout are
            // indistinguishable here, and reporting the counterpart gone while
            // its answer is already published would be a race, not a diagnosis.
            if seq.load(std::sync::atomic::Ordering::SeqCst) == last && !keep_waiting() {
                return None;
            }
            spun = 0;
        }
    }

    fn send_request(&self, seq: u32) {
        self.publish(OFF_REQ_SEQ, OFF_REQ_SLEEPERS, seq);
    }

    /// The worker's wait for the next call. Untimed: a worker has its own way of
    /// noticing the caller is gone (`PR_SET_PDEATHSIG`), so waking it on a timer
    /// would burn a wakeup per idle period to learn nothing.
    fn await_request(&self, last: u32) -> u32 {
        self.await_past(OFF_REQ_SEQ, OFF_REQ_SLEEPERS, last, None, || true)
            .expect("an untimed wait never abandons")
    }

    fn send_response(&self, seq: u32) {
        self.publish(OFF_RESP_SEQ, OFF_RESP_SLEEPERS, seq);
    }

    /// The caller's wait for an answer, abandoned when `alive` reports the
    /// worker has gone. The caller has no signal for a child's death — it has to
    /// look — so this is the side that polls.
    fn await_response<F: FnMut() -> bool>(&self, last: u32, alive: F) -> Option<u32> {
        self.await_past(
            OFF_RESP_SEQ,
            OFF_RESP_SLEEPERS,
            last,
            Some(LIVENESS_POLL),
            alive,
        )
    }
}

// ── futex ───────────────────────────────────────────────────────────────────
//
// Shared (no `FUTEX_PRIVATE_FLAG`): the word lives in a file mapping that spans
// two processes, and the private variant hashes on the mm, so the two sides
// would queue on different keys and every wake would be lost.

fn futex_wait(a: &std::sync::atomic::AtomicU32, expect: u32, limit: Option<std::time::Duration>) {
    let ts = limit.map(|d| libc::timespec {
        tv_sec: d.as_secs() as libc::time_t,
        tv_nsec: libc::c_long::from(d.subsec_nanos()),
    });
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            std::ptr::from_ref(a),
            libc::FUTEX_WAIT,
            expect,
            ts.as_ref()
                .map_or(std::ptr::null(), std::ptr::from_ref::<libc::timespec>),
        );
    }
}

fn futex_wake(a: &std::sync::atomic::AtomicU32) {
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            std::ptr::from_ref(a),
            libc::FUTEX_WAKE,
            1i32,
        );
    }
}

// ── frame codec ─────────────────────────────────────────────────────────────

/// A bounds-checked cursor over the payload region.
///
/// Every read is checked against the region end: a worker must not be
/// trustable into reading past the mapping because a length word was wrong, and
/// a length word is exactly what a mismatched build gets wrong.
struct Frame {
    base: *mut u8,
    at: usize,
    cap: usize,
}

/// How much of the mapping a frame may use.
const PAYLOAD_BYTES: usize = WIRE_BYTES - OFF_PAYLOAD;

/// The `pos` a text answer parked in the return arena is handed back with
/// (loft#1061).  A text handle is a RECORD number, so `rec` carries it and `pos`
/// has nothing to say; this names the reference as one rather than leaving a
/// zero that reads like an ordinary record's first field.
pub(crate) const TEXT_IN_ARENA: u32 = u32::MAX;

/// Can this text be marshalled inline, or must it go through the arena?
///
/// The margin covers the response header (`RESP_OK`, both arena sizes, the tag and
/// the length) with room to spare — the exact byte count is not worth pinning, and
/// being an answer or two conservative only moves a large text onto the arena path,
/// which is correct for either.
fn fits_in_frame(s: &str) -> bool {
    s.len() + 64 < PAYLOAD_BYTES
}

impl Frame {
    fn new(base: *mut u8) -> Frame {
        Frame {
            base,
            at: 0,
            cap: PAYLOAD_BYTES,
        }
    }

    /// A frame over an ordinary buffer — what a `remote` crossing builds into,
    /// since it has no shared mapping to write through. The buffer must already
    /// be `PAYLOAD_BYTES` long; the frame reports how much of it was used.
    fn over(buf: &mut [u8]) -> Frame {
        Frame {
            base: buf.as_mut_ptr(),
            at: 0,
            cap: buf.len(),
        }
    }

    /// How many bytes this frame has written or read.
    fn len(&self) -> usize {
        self.at
    }

    fn room(&self, n: usize) -> bool {
        self.at + n <= self.cap
    }

    fn put_u32(&mut self, v: u32) -> bool {
        if !self.room(4) {
            return false;
        }
        unsafe {
            self.base.add(self.at).cast::<u32>().write_unaligned(v);
        }
        self.at += 4;
        true
    }

    fn get_u32(&mut self) -> Option<u32> {
        if !self.room(4) {
            return None;
        }
        let v = unsafe { self.base.add(self.at).cast::<u32>().read_unaligned() };
        self.at += 4;
        Some(v)
    }

    fn put_i64(&mut self, v: i64) -> bool {
        if !self.room(8) {
            return false;
        }
        unsafe {
            self.base.add(self.at).cast::<i64>().write_unaligned(v);
        }
        self.at += 8;
        true
    }

    fn get_i64(&mut self) -> Option<i64> {
        if !self.room(8) {
            return None;
        }
        let v = unsafe { self.base.add(self.at).cast::<i64>().read_unaligned() };
        self.at += 8;
        Some(v)
    }

    fn put_str(&mut self, s: &str) -> bool {
        let b = s.as_bytes();
        if !self.put_u32(b.len() as u32) || !self.room(b.len()) {
            return false;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(b.as_ptr(), self.base.add(self.at), b.len());
        }
        self.at += b.len();
        true
    }

    /// Write a fault: the detail, its kind label, its position and its frames.
    ///
    /// Every part is what RENDERS — `RuntimeError::to_diag_entry` and
    /// `render_call_chain` read exactly these — so a fault that survives this
    /// round trip renders identically to the same fault in-process, which is the
    /// gate `placement_parity` holds it to.
    fn put_fault(&mut self, e: &Fault) -> bool {
        let (file, line, col) = e
            .position
            .as_ref()
            .map_or(("", 0u32, 0u32), |p| (&*p.file, p.line, p.pos));
        self.put_str(&e.message)
            && self.put_str(&e.label)
            && self.put_str(file)
            && self.put_u32(line)
            && self.put_u32(col)
            && self.put_u32(e.call_chain.len() as u32)
            && e.call_chain.iter().all(|n| self.put_str(n))
    }

    /// Read a fault back.  A frame too short to hold one is itself a fault, and
    /// says so rather than answering a half-decoded one.
    fn get_fault(&mut self) -> Option<Fault> {
        let message = self.get_str()?;
        let label = self.get_str()?;
        let file = self.get_str()?;
        let line = self.get_u32()?;
        let col = self.get_u32()?;
        let n = self.get_u32()? as usize;
        let mut call_chain = Vec::with_capacity(n);
        for _ in 0..n {
            call_chain.push(self.get_str()?);
        }
        Some(Fault {
            message,
            label,
            // An empty file is "no position known", the same reading
            // `RuntimeError::user_panic` gives it.
            position: (!file.is_empty()).then_some(crate::lexer::Position {
                file: file.into(),
                line,
                pos: col,
            }),
            call_chain,
        })
    }

    fn get_str(&mut self) -> Option<String> {
        let n = self.get_u32()? as usize;
        if !self.room(n) {
            return None;
        }
        let s = unsafe { std::slice::from_raw_parts(self.base.add(self.at), n) };
        self.at += n;
        // Lossy rather than fatal: the bytes came from a loft `text`, which is
        // already UTF-8, so a failure here means a corrupt frame — and reporting
        // it as a garbled name gives a better error than a decode panic.
        Some(String::from_utf8_lossy(s).into_owned())
    }

    fn put_value(&mut self, v: &crate::host::Value) -> bool {
        use crate::host::Value;
        match v {
            Value::Void => self.put_u32(TAG_VOID),
            Value::Bool(b) => self.put_u32(TAG_BOOL) && self.put_i64(i64::from(*b)),
            Value::Int(i) => self.put_u32(TAG_INT) && self.put_i64(*i),
            Value::Float(f) => self.put_u32(TAG_FLOAT) && self.put_i64(f.to_bits() as i64),
            Value::Text(s) => self.put_u32(TAG_TEXT) && self.put_str(s),
            Value::Ref(r) if r.is_null() => self.put_u32(TAG_NULLREF),
            Value::Ref(r) => self.put_u32(TAG_REF) && self.put_u32(r.rec) && self.put_u32(r.pos),
        }
    }

    /// Decode one value.  `arena_nr` is the store number THIS side registered the
    /// call arena under, and is what a `TAG_REF` is completed with.
    fn get_value(&mut self, arena_nr: u16) -> Option<crate::host::Value> {
        use crate::host::Value;
        match self.get_u32()? {
            TAG_VOID => Some(Value::Void),
            TAG_BOOL => Some(Value::Bool(self.get_i64()? != 0)),
            TAG_INT => Some(Value::Int(self.get_i64()?)),
            TAG_FLOAT => Some(Value::Float(f64::from_bits(self.get_i64()? as u64))),
            TAG_TEXT => Some(Value::Text(self.get_str()?)),
            TAG_NULLREF => Some(Value::Ref(crate::keys::DbRef::NULL)),
            TAG_REF => {
                let rec = self.get_u32()?;
                let pos = self.get_u32()?;
                Some(Value::Ref(crate::keys::DbRef {
                    store_nr: arena_nr,
                    rec,
                    pos,
                }))
            }
            _ => None,
        }
    }
}

// ── caller side ─────────────────────────────────────────────────────────────

/// A library running in a worker process, and the wire to reach it.
///
/// One per process-placed library, created at load and kept for the run: the
/// worker holds the library's stores across calls, which is what lets a placed
/// library keep state exactly as an in-process one does.
pub struct Worker {
    /// How the frame and the arenas reach the other side — the ONE thing that
    /// differs between `process` and `remote` (@PLN119 arc E).
    ///
    /// Everything else about a crossing — the marshal, the layout gate, the
    /// delivery three-way, the `const` skip, the fault handling — is a property
    /// of the BOUNDARY rather than of the wire, and is shared verbatim.
    link: Link,
    /// The call arena, one store per direction (@PLN119 arc B).
    ///
    /// `arg` is written by the caller and `ret` by the worker — one writer each,
    /// because a `Store` caches its free list on the Rust side and two
    /// processes allocating out of one would hand out the same block twice.
    ///
    /// Under `remote` nobody else maps these; they are this side's scratch, and
    /// their BYTES travel instead ([`Arena::image`]).
    arg: Arena,
    ret: Arena,
    /// Last request sequence sent. Requests and responses share the counter so
    /// a response can be matched to its request.
    seq: std::cell::Cell<u32>,
    /// Set once the worker has been seen to die, so every later call reports
    /// the death instead of blocking forever on a wire nobody is serving.
    dead: std::cell::Cell<bool>,
    name: String,
}

/// The two ways a crossing reaches the other side.
///
/// A `process` placement shares memory with a child of this process and signals
/// it with a futex; a `remote` one owns nothing but a socket, and the arenas
/// travel as bytes on it. The split is here — one enum, two arms — rather than
/// spread through the dispatcher, because the plan's invariant is that placement
/// is deployment policy: if the two transports had two boundaries, they would be
/// two programs.
enum Link {
    /// A worker process of this machine: a shared mapping plus the child.
    Local {
        wire: Wire,
        /// Behind a `RefCell` so a call — which holds `&self` — can ask whether
        /// the child is still running. `try_wait` reaps, which needs `&mut`.
        child: std::cell::RefCell<std::process::Child>,
    },
    /// A server, reachable at an address. Nothing is shared and nothing is
    /// owned: this side did not start it and does not stop it.
    Remote {
        stream: std::cell::RefCell<std::net::TcpStream>,
        address: String,
        /// The last response's payload, kept so `reread_answer` can decode the
        /// compound reference after the arenas are bound — the same two-step the
        /// local transport does against the shared mapping.
        last: std::cell::RefCell<Vec<u8>>,
    },
}

impl Drop for Worker {
    fn drop(&mut self) {
        match &self.link {
            Link::Local { wire, child } => {
                if !self.dead.get() {
                    let seq = self.seq.get() + 1;
                    let mut f = Frame::new(wire.payload());
                    if f.put_u32(REQ_SHUTDOWN) {
                        wire.send_request(seq);
                    }
                }
                // A worker that ignores the shutdown word must not wedge the run.
                let mut child = child.borrow_mut();
                let _ = child.kill();
                let _ = child.wait();
            }
            // A remote server outlives its callers by design — closing the
            // socket is the whole goodbye, and killing it would be someone
            // else's process.
            Link::Remote { .. } => {}
        }
    }
}

impl Worker {
    /// Start a worker holding the library at `pkg_dir` and complete the attach
    /// handshake.
    ///
    /// # Errors
    /// A failure to create the wire, spawn the worker, or hear its ready
    /// response — each reported with the library name, because the operator's
    /// next question is always which library failed to place.
    /// `cwd` is the directory the CALLER will be running in — not necessarily
    /// the one it is in now.
    ///
    /// loft anchors a program's relative file access at its own source directory
    /// and chdirs there before running, but that happens long after libraries are
    /// installed. A worker started before it would inherit the INVOCATION
    /// directory, and then every relative path a placed library touched would
    /// resolve somewhere else than the same library in-process — a divergence
    /// with no error, found by `lib/git` answering "not a git repository" in one
    /// placement and the history in the other.
    pub fn spawn(name: &str, pkg_dir: &Path, stdlib_dir: &Path, cwd: &Path) -> io::Result<Worker> {
        let path = std::env::temp_dir().join(format!(
            "loft-place-{}-{}.wire",
            std::process::id(),
            name.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
        ));
        let wire = Wire::create(&path)?;
        // Both arenas exist and are initialised before the worker starts, so
        // there is no window in which it can attach a half-built store.
        let (arg_path, ret_path) = arena_paths(&path);
        let arg = Arena::create(&arg_path)?;
        let ret = Arena::create(&ret_path)?;
        // Normally the worker is this same binary. `LOFT_WORKER_EXE` names a
        // different one, which is what lets a test drive a real worker without
        // being the `loft` executable itself — and what lets an embedder whose
        // process is not `loft` place a library at all.
        let exe = match std::env::var_os("LOFT_WORKER_EXE") {
            Some(p) => PathBuf::from(p),
            None => std::env::current_exe()?,
        };
        let mut command = std::process::Command::new(exe);
        command
            .arg("--lib-worker")
            .arg(&path)
            .arg(pkg_dir)
            .arg("--default")
            .arg(stdlib_dir);
        if !cwd.as_os_str().is_empty() {
            command.current_dir(cwd);
        }
        let child = command.spawn()?;

        let mut w = Worker {
            link: Link::Local {
                wire,
                child: std::cell::RefCell::new(child),
            },
            arg,
            ret,
            seq: std::cell::Cell::new(0),
            dead: std::cell::Cell::new(false),
            name: name.to_string(),
        };
        // The handshake is the first call: the worker answers it only after the
        // library has parsed and compiled, so a library that cannot load is
        // reported here rather than at whichever call happens to be first.
        match w.call_raw("", &[]) {
            Ok(_) => Ok(w),
            Err(e) => Err(io::Error::other(format!(
                "library '{name}' declares placement = \"process\" but its worker did not \
                 come up: {e}"
            ))),
        }
    }

    /// @PLN119 arc E — reach a library that is already running somewhere else.
    ///
    /// Nothing is spawned and nothing is owned: the server is started by whoever
    /// deploys it, and this side only connects. That asymmetry with
    /// [`spawn`](Worker::spawn) is the honest one — a local worker is this
    /// process's to start and to kill, and a remote one never is.
    ///
    /// # Errors
    /// A failure to reach the address, or to hear the ready response — each
    /// reported with the library name, because the operator's next question is
    /// always which library failed to place.
    pub fn connect(name: &str, address: &str) -> io::Result<Worker> {
        let stream = crate::net_profile::time("placement/connect", None, || {
            std::net::TcpStream::connect(address)
        })
        .map_err(|e| {
            io::Error::other(format!(
                "library '{name}' declares placement = \"remote\" but {address} could \
                 not be reached: {e}"
            ))
        })?;
        // A crossing is a request and its answer; batching them would trade a
        // round trip for a latency spike on every call that fits in one packet,
        // which is every call this carries.
        let _ = stream.set_nodelay(true);
        // The arenas are ordinary local scratch here — nobody else maps them,
        // and it is their BYTES that travel.
        let path = std::env::temp_dir().join(format!(
            "loft-remote-{}-{}.wire",
            std::process::id(),
            name.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
        ));
        let (arg_path, ret_path) = arena_paths(&path);
        let arg = Arena::create(&arg_path)?;
        let ret = Arena::create(&ret_path)?;

        let mut w = Worker {
            link: Link::Remote {
                stream: std::cell::RefCell::new(stream),
                address: address.to_string(),
                last: std::cell::RefCell::new(Vec::new()),
            },
            arg,
            ret,
            seq: std::cell::Cell::new(0),
            dead: std::cell::Cell::new(false),
            name: name.to_string(),
        };
        match w.call_raw("", &[]) {
            Ok(_) => Ok(w),
            Err(e) => Err(io::Error::other(format!(
                "library '{name}' at {address} did not answer: {e}"
            ))),
        }
    }

    /// The store-numbering base the worker reported — arc B translates against
    /// it. Present here so attaching does not change when arc B lands.
    ///
    /// Zero for a remote placement, which shares no store numbering at all.
    #[must_use]
    pub fn store_base(&self) -> u32 {
        match &self.link {
            Link::Local { wire, .. } => wire.store_base(),
            Link::Remote { .. } => 0,
        }
    }

    /// The worker process's id — what a caller needs to observe or signal it.
    /// Zero when there is no local process to name.
    ///
    /// # Panics
    /// If the handle is already borrowed, which only happens while this worker
    /// is being dropped.
    #[must_use]
    pub fn child_id(&self) -> u32 {
        match &self.link {
            Link::Local { child, .. } => child.borrow().id(),
            Link::Remote { .. } => 0,
        }
    }

    /// Is the worker still there?
    ///
    /// `try_wait` rather than a signal probe, because a worker that has exited
    /// but not been reaped is a zombie — still a live pid, answering `kill(0)`
    /// perfectly happily, and never going to serve another call.
    fn still_running(&self) -> bool {
        match &self.link {
            Link::Local { child, .. } => match child.try_borrow_mut() {
                Ok(mut c) => matches!(c.try_wait(), Ok(None)),
                // Borrowed means `Drop` is already tearing this worker down; let
                // the wait end rather than claim a liveness we cannot check.
                Err(_) => false,
            },
            // A socket answers this question by failing to read, which the
            // remote exchange already surfaces as the call's error.
            Link::Remote { .. } => true,
        }
    }

    /// The arena a compound ARGUMENT is marshalled into. Written by this side.
    pub fn arg_arena(&mut self) -> &mut Arena {
        &mut self.arg
    }

    /// The arena a compound RETURN comes back in. Written by the worker; this
    /// side only reads it, and only until the next call resets it.
    pub fn ret_arena(&mut self) -> &mut Arena {
        &mut self.ret
    }

    /// How the WORKER lays out `func`'s compound parameters and return.
    ///
    /// The caller compares this against its own answer and refuses to place a
    /// function the two disagree about. See [`super::dispatch`]'s layout gate.
    ///
    /// # Errors
    /// A transport failure, or the worker not knowing the function.
    pub fn layout(&mut self, func: &str) -> Result<String, String> {
        let mut buf = vec![0u8; PAYLOAD_BYTES];
        let mut f = Frame::over(&mut buf);
        if !(f.put_u32(REQ_LAYOUT) && f.put_str(func)) {
            return Err("layout request does not fit the wire".to_string());
        }
        let used = f.len();
        buf.truncate(used);
        match self.exchange(&buf, func).map_err(|e| e.message)? {
            crate::host::Value::Text(s) => Ok(s),
            other => Err(format!("layout answer was {other:?}")),
        }
    }

    /// Call `func` in the worker.
    ///
    /// A COMPOUND answer comes back as a reference this side cannot complete
    /// yet — the return arena has no store number until it is bound, and it
    /// cannot be bound until it has been re-mapped, which this call is what
    /// decides. So the reference is left arena-relative here and
    /// [`reread_answer`](Worker::reread_answer) completes it once the caller has
    /// bound the arena.
    ///
    /// # Errors
    /// The worker's own error text for a fault inside the library, or a
    /// transport failure (a dead worker, an oversized frame).
    pub fn call(
        &mut self,
        func: &str,
        args: &[crate::host::Value],
    ) -> Result<crate::host::Value, Fault> {
        if func.is_empty() {
            return Err("empty function name".into());
        }
        self.call_raw(func, args)
    }

    /// The handshake and every later call take the same path; `func == ""` is
    /// the handshake, which the worker answers with void once it is loaded.
    fn call_raw(
        &mut self,
        func: &str,
        args: &[crate::host::Value],
    ) -> Result<crate::host::Value, Fault> {
        // The arg arena's size travels ahead of the arguments: marshalling may
        // have grown the file, and the worker has to map the new length before
        // it can read a record that lives past the old end.
        let arg_words = self.arg.words();
        let mut buf = vec![0u8; PAYLOAD_BYTES];
        let mut f = Frame::over(&mut buf);
        let built = f.put_u32(REQ_CALL)
            && f.put_u32(arg_words)
            && f.put_str(func)
            && f.put_u32(args.len() as u32)
            && args.iter().all(|a| f.put_value(a));
        if !built {
            return Err(format!(
                "call to '{func}' does not fit the placement wire ({} KiB) — \
                 arc B sizes the arena from the argument graph",
                WIRE_BYTES / 1024
            )
            .into());
        }
        let used = f.len();
        buf.truncate(used);
        self.exchange(&buf, func)
    }

    /// Send `request`, wait for the answer, bring both arenas up to date, and
    /// decode.
    ///
    /// The two transports differ in exactly what "send" and "bring up to date"
    /// mean, and in nothing else — a local crossing writes through a shared
    /// mapping the other side already has, a remote one puts the arena's bytes
    /// on the socket beside the frame. Everything after that point is common,
    /// which is why the arms below are short.
    fn exchange(&mut self, request: &[u8], func: &str) -> Result<crate::host::Value, Fault> {
        if self.dead.get() {
            return Err(format!("library '{}' is gone", self.name).into());
        }
        let seq = self.seq.get() + 1;
        self.seq.set(seq);
        match &self.link {
            Link::Local { .. } => self.exchange_local(request, func, seq),
            Link::Remote { .. } => self.exchange_remote(request, func),
        }
    }

    /// The shared-mapping crossing: copy the frame in, publish, wait, and let
    /// the arenas re-map themselves — the other side wrote into the very files
    /// this one has open.
    fn exchange_local(
        &mut self,
        request: &[u8],
        func: &str,
        seq: u32,
    ) -> Result<crate::host::Value, Fault> {
        let Link::Local { wire, .. } = &self.link else {
            unreachable!("dispatched on the link")
        };
        unsafe {
            std::ptr::copy_nonoverlapping(request.as_ptr(), wire.payload(), request.len());
        }
        wire.send_request(seq);
        if wire
            .await_response(seq - 1, || self.still_running())
            .is_none()
        {
            // The worker died with this call outstanding. Nothing will ever
            // write the response word, so the wait had to end by looking rather
            // than by being woken. Remember it: the wire is unusable from here,
            // and a later call must say so at once instead of waiting again.
            self.dead.set(true);
            return Err(format!(
                "library '{}' worker died during the call to '{}'",
                self.name,
                if func.is_empty() { "<load>" } else { func }
            )
            .into());
        }
        let mut f = Frame::new(wire.payload());
        let (ret_words, arg_words) = read_answer_header(&mut f, &self.name)?;
        self.arg
            .remap_if_grown(arg_words)
            .map_err(|e| format!("call arena (arguments) of '{}': {e}", self.name))?;
        self.ret
            .remap_if_grown(ret_words)
            .map_err(|e| format!("call arena (return) of '{}': {e}", self.name))?;
        f.get_value(u16::MAX)
            .ok_or_else(|| Fault::from(format!("malformed response frame from '{}'", self.name)))
    }

    /// @PLN119 arc E — the socket crossing.
    ///
    /// The arg arena travels WITH the request and comes back WITH the answer,
    /// and both directions are load-bearing: loft passes a compound by
    /// reference, so a callee's write to a parameter is the caller's to see, and
    /// over a socket the only way it can be seen is for the bytes to come home.
    fn exchange_remote(&mut self, request: &[u8], func: &str) -> Result<crate::host::Value, Fault> {
        let response = {
            let Link::Remote { stream, .. } = &self.link else {
                unreachable!("dispatched on the link")
            };
            let mut stream = stream.borrow_mut();
            let arg_image = self.arg.image();
            if let Err(e) = put_message(&mut *stream, &[request, arg_image]) {
                self.dead.set(true);
                return Err(self.gone(func, &e).into());
            }
            match get_message(&mut *stream, 3) {
                Ok(three) => three,
                Err(e) => {
                    self.dead.set(true);
                    return Err(self.gone(func, &e).into());
                }
            }
        };
        let mut parts = response.into_iter();
        let mut body = parts.next().unwrap_or_default();
        let ret_image = parts.next().unwrap_or_default();
        let arg_image = parts.next().unwrap_or_default();
        // Adopt the answer's arenas before decoding, so a compound reference
        // names a record that is already here.
        self.ret.load_image(&ret_image);
        self.arg.load_image(&arg_image);

        let mut f = Frame::over(&mut body);
        let _ = read_answer_header(&mut f, &self.name)?;
        let value = f
            .get_value(u16::MAX)
            .ok_or_else(|| Fault::from(format!("malformed response frame from '{}'", self.name)))?;
        if let Link::Remote { last, .. } = &self.link {
            *last.borrow_mut() = body;
        }
        Ok(value)
    }

    /// The message for a remote link that stopped answering. Named separately
    /// because it is the one failure a remote placement has that a local one
    /// does not: the far side is someone else's process on someone else's
    /// machine, and "it died" is a guess where "it stopped answering" is a fact.
    fn gone(&self, func: &str, why: &str) -> String {
        let where_ = match &self.link {
            Link::Remote { address, .. } => address.clone(),
            Link::Local { .. } => "this machine".to_string(),
        };
        format!(
            "library '{}' at {where_} stopped answering during the call to '{}': {why}",
            self.name,
            if func.is_empty() { "<load>" } else { func }
        )
    }

    /// Re-read the answer, now that the caller knows which store number to
    /// complete a compound reference with.
    ///
    /// Only meaningful immediately after a successful [`call`](Worker::call), and
    /// before the next request replaces it.
    pub fn reread_answer(&self, arena_nr: u16) -> Option<crate::host::Value> {
        match &self.link {
            Link::Local { wire, .. } => {
                let mut f = Frame::new(wire.payload());
                read_answer_header(&mut f, &self.name).ok()?;
                f.get_value(arena_nr)
            }
            Link::Remote { last, .. } => {
                let mut body = last.borrow_mut();
                let mut f = Frame::over(&mut body);
                read_answer_header(&mut f, &self.name).ok()?;
                f.get_value(arena_nr)
            }
        }
    }
}

/// Read a response's status and the two arena sizes, which every answer carries
/// before its value.
///
/// The sizes come back on the ERROR path too: the callee may have grown the
/// argument arena before it faulted, and a caller that read the old size would
/// then map short and read a hole where its own value was.
fn read_answer_header(f: &mut Frame, name: &str) -> Result<(u32, u32), Fault> {
    let status = f.get_u32();
    let ret_words = f.get_u32().unwrap_or(0);
    let arg_words = f.get_u32().unwrap_or(0);
    match status {
        Some(RESP_OK) => Ok((ret_words, arg_words)),
        Some(RESP_ERR) => Err(f
            .get_fault()
            .unwrap_or_else(|| Fault::from("malformed error frame"))),
        _ => Err(Fault::from(format!(
            "malformed response frame from '{name}'"
        ))),
    }
}

/// Write every part of one crossing as a SINGLE message.
///
/// One `write_all` rather than one per part, and the reason is the same one that
/// shaped the local handshake (@PLN119 Q4): the obvious implementation is the
/// slow one. Three small writes on a socket are three syscalls and, with
/// `TCP_NODELAY` on, three segments — measured, roughly a fifth of a loopback
/// crossing's cost for parts that all belong to the same message anyway.
fn put_message(w: &mut impl std::io::Write, parts: &[&[u8]]) -> Result<(), String> {
    let body: usize = parts.iter().map(|p| p.len() + 4).sum();
    let total = u32::try_from(body).map_err(|_| "message too large".to_string())?;
    let mut msg = Vec::with_capacity(body + 4);
    msg.extend_from_slice(&total.to_le_bytes());
    for p in parts {
        msg.extend_from_slice(&(p.len() as u32).to_le_bytes());
        msg.extend_from_slice(p);
    }
    crate::net_profile::time("placement/write", None, || w.write_all(&msg))
        .map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())
}

/// Read one message and split it back into its parts.
///
/// The total length is checked against a ceiling BEFORE anything is allocated:
/// this side is reading from a socket, and a length word is exactly what a wrong
/// or hostile peer gets wrong. A mistyped port number should be a refusal, not
/// an out-of-memory kill.
fn get_message(r: &mut impl std::io::Read, parts: usize) -> Result<Vec<Vec<u8>>, String> {
    let mut len = [0u8; 4];
    crate::net_profile::time("placement/read_len", None, || r.read_exact(&mut len))
        .map_err(|e| e.to_string())?;
    let total = u32::from_le_bytes(len) as usize;
    if total > MAX_MESSAGE_BYTES {
        return Err(format!(
            "message of {total} bytes exceeds the {MAX_MESSAGE_BYTES}-byte placement limit"
        ));
    }
    let mut buf = vec![0u8; total];
    crate::net_profile::time("placement/read_body", None, || r.read_exact(&mut buf))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(parts);
    let mut at = 0usize;
    for _ in 0..parts {
        if at + 4 > buf.len() {
            return Err("truncated placement message".to_string());
        }
        let n = u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]) as usize;
        at += 4;
        if at + n > buf.len() {
            return Err("truncated placement message".to_string());
        }
        out.push(buf[at..at + n].to_vec());
        at += n;
    }
    Ok(out)
}

/// The largest single message a placement link will read.
///
/// A ceiling rather than trust: the length word arrives from another machine,
/// and allocating what it asks for is how a mistyped port number becomes an
/// out-of-memory kill. Generous enough for a real value graph — the arenas in
/// @PLN119's own tests carry 200 000 records — and far below what a wrong number
/// would ask for.
const MAX_MESSAGE_BYTES: usize = 256 << 20;

// ── worker side ─────────────────────────────────────────────────────────────

/// Run as the worker for one placed library: attach the wire, load the library,
/// then serve calls until the caller says stop or goes away.
///
/// Entered from `loft --lib-worker <wire> <pkg_dir> --default <stdlib>`. Never
/// returns normally — it exits the process, because a worker that fell out of
/// its loop has nothing else to be.
///
/// # Panics
/// Never deliberately; a fault inside the library is caught and returned to the
/// caller as an error, which is what makes a placed library's error behaviour
/// match an in-process one.
pub fn serve(wire_path: &Path, pkg_dir: &Path, stdlib_dir: &Path) -> ! {
    // Die with the caller. A worker is meaningless without the process that
    // started it, and the caller cannot be relied on to say goodbye: it may
    // `exit` from any of a dozen places, or be killed outright. Without this a
    // crashed run leaves a worker holding the terminal's stdout, which reads as
    // the run itself having hung.
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
    }
    // Re-check after arming: if the caller died in the window before the
    // `prctl`, the signal has already been missed.
    if unsafe { libc::getppid() } == 1 {
        std::process::exit(0);
    }
    let wire = match Wire::attach(wire_path) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("loft worker: cannot attach {}: {e}", wire_path.display());
            std::process::exit(1);
        }
    };

    let loaded = crate::host::Program::from_library_dir(pkg_dir, stdlib_dir);

    // Report the store-numbering base before answering the handshake, so the
    // caller never observes a ready worker without one.
    let mut program = match loaded {
        Ok(mut p) => {
            // Resolve a relative path the way the CALLER does. The caller spawned
            // this worker in the directory it will itself run in, so that
            // directory — not the library's own — is what `file("x")` means to
            // the code this worker serves.
            if let Ok(here) = std::env::current_dir() {
                p.anchor_paths_at(&here);
            }
            wire.set_store_base(p.store_count());
            wire.set_epoch(p.store_epoch());
            Some(p)
        }
        Err(e) => {
            // Keep serving: the handshake reply carries the load error, which
            // is how the caller reports it against the library's name.
            wire.set_store_base(0);
            let mut f = Frame::new(wire.payload());
            let _ = f.put_u32(RESP_ERR) && f.put_fault(&Fault::from(e.to_string()));
            wire.send_response(1);
            std::process::exit(0);
        }
    };

    // The arenas the caller created before starting us. A worker that cannot map
    // them answers the handshake with the reason, exactly as a library that
    // cannot parse does — a worker that came up half-attached would fail at
    // whichever call first carried a struct.
    let (arg_path, ret_path) = arena_paths(wire_path);
    let mut arenas = match (Arena::attach(&arg_path), Arena::attach(&ret_path)) {
        (Ok(a), Ok(r)) => (a, r),
        (Err(e), _) | (_, Err(e)) => {
            let mut f = Frame::new(wire.payload());
            let _ = f.put_u32(RESP_ERR)
                && f.put_u32(0)
                && f.put_u32(0)
                && f.put_fault(&Fault::from(format!("cannot map the call arena: {e}")));
            wire.send_response(1);
            std::process::exit(0);
        }
    };

    let mut last = 0u32;
    loop {
        last = wire.await_request(last);
        let mut f = Frame::new(wire.payload());
        let kind = f.get_u32();
        if kind == Some(REQ_SHUTDOWN) || kind.is_none() {
            std::process::exit(0);
        }
        let reply = if kind == Some(REQ_LAYOUT) {
            answer_layout(&mut f, program.as_mut())
        } else {
            serve_one(&mut f, program.as_mut(), &mut arenas)
        };
        let (arg_words, ret_words) = (arenas.0.words(), arenas.1.words());
        let mut out = Frame::new(wire.payload());
        // The sizes go out on the error path too: the callee may have grown the
        // ARGUMENT arena before it faulted, and a caller that read the old size
        // would then map short and read a hole where its own value was.
        match reply {
            Ok(v) => {
                if !(out.put_u32(RESP_OK)
                    && out.put_u32(ret_words)
                    && out.put_u32(arg_words)
                    && out.put_value(&v))
                {
                    let mut out = Frame::new(wire.payload());
                    let _ = out.put_u32(RESP_ERR)
                        && out.put_u32(ret_words)
                        && out.put_u32(arg_words)
                        && out.put_fault(&Fault::from(
                            "return value does not fit the placement wire",
                        ));
                }
            }
            Err(e) => {
                let mut out = Frame::new(wire.payload());
                let _ = out.put_u32(RESP_ERR)
                    && out.put_u32(ret_words)
                    && out.put_u32(arg_words)
                    && out.put_fault(&e);
            }
        }
        wire.send_response(last);
    }
}

/// Report how THIS program lays out a function's compound types, for the
/// caller's layout gate.
fn answer_layout(
    f: &mut Frame,
    program: Option<&mut crate::host::Program>,
) -> Result<crate::host::Value, Fault> {
    let func = f.get_str().ok_or("malformed layout request")?;
    let program = program.ok_or("library failed to load")?;
    Ok(crate::host::Value::Text(
        crate::lib_placement::dispatch::signature_layout(program, &func)
            .ok_or_else(|| format!("no function '{func}'"))?,
    ))
}

/// Decode one call frame and run it against the loaded library, with the arenas
/// bound for the duration.
///
/// The order here is the contract, and each step is load-bearing:
///
/// 1. **Re-map** the argument arena if the caller's marshal grew the file — its
///    records may live past the length this process mapped.
/// 2. **Resync** it. loft passes a compound BY REFERENCE, so the callee may
///    append to a vector parameter and allocate in this arena; allocating
///    against a free list cached before the caller's claims would hand out a
///    block that is already in use.
/// 3. **Reset** the return arena. This side owns it, and last call's answer is
///    dead the moment this one starts.
fn serve_one(
    f: &mut Frame,
    program: Option<&mut crate::host::Program>,
    arenas: &mut (Arena, Arena),
) -> Result<crate::host::Value, Fault> {
    let arg_words = f.get_u32().ok_or("malformed request frame")?;
    let program = program.ok_or("library failed to load")?;
    arenas
        .0
        .remap_if_grown(arg_words)
        .map_err(|e| format!("cannot map the argument arena: {e}"))?;
    arenas.0.resync();
    serve_bound(f, program, arenas)
}

/// The half of serving a call that is the same on every transport: bind both
/// arenas, run, unbind.
///
/// The argument arena is already this side's to read — a shared mapping the
/// caller wrote through, or an image it sent — and the return arena is reset
/// here because this side owns it and last call's answer is dead.
fn serve_bound(
    f: &mut Frame,
    program: &mut crate::host::Program,
    arenas: &mut (Arena, Arena),
) -> Result<crate::host::Value, Fault> {
    arenas.1.reset();
    let arg_nr = arenas.0.bind(program.stores());
    let ret_nr = arenas.1.bind(program.stores());
    let out = run_bound(f, program, ret_nr, arg_nr);
    arenas.0.unbind(program.stores(), arg_nr);
    arenas.1.unbind(program.stores(), ret_nr);
    out
}

/// The part of a call that runs while both arenas are registered.
fn run_bound(
    f: &mut Frame,
    program: &mut crate::host::Program,
    ret_nr: u16,
    arg_nr: u16,
) -> Result<crate::host::Value, Fault> {
    let func = f.get_str().ok_or("malformed request frame")?;
    let n_args = f.get_u32().ok_or("malformed request frame")? as usize;
    let mut args = Vec::with_capacity(n_args);
    for _ in 0..n_args {
        args.push(
            f.get_value(arg_nr)
                .ok_or("malformed argument in request frame")?,
        );
    }
    // The handshake: the caller only needs to know we got here.
    if func.is_empty() {
        return Ok(crate::host::Value::Void);
    }
    for a in &args {
        if let crate::host::Value::Ref(r) = a {
            super::arena::trace(program.stores(), "worker-arg", r, u16::MAX);
        }
    }
    // A compound answer is BUILT in the return arena rather than copied into it:
    // the destination a loft function's hidden `__retbuf` parameter names is
    // simply a record over there. A callee that ignores the offer and mints its
    // own store is the other half of the same protocol, handled below.
    let ret_ty = program
        .signature(&func)
        .ok_or_else(|| format!("no function '{func}'"))?
        .1;
    let retbuf = if crate::host::is_compound(&ret_ty) {
        let (tp, _) = program.layout_of(&ret_ty);
        super::arena::alloc_value(program.stores(), ret_nr, tp)
    } else {
        crate::keys::DbRef::NULL
    };

    // A fault inside the library is the caller's error, not the worker's death
    // — arc D extends this to a worker that dies outright.
    let out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        program.call_into(&func, &args, retbuf)
    })) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return Err(Fault::from_loft_error(&e)),
        Err(_) => return Err(Fault::from(format!("library panicked in '{func}'"))),
    };

    // loft#1061 — a TEXT answer bigger than the frame travels in the return arena,
    // the growable channel a compound answer already uses.  `Value::Text` is
    // marshalled inline (`put_str`), so the fixed 1 MiB frame was a ceiling on how
    // long a placed function's text answer could be: `git::show(sha)` on a 3.7 MB
    // commit reached the caller as `panic: return value does not fit the placement
    // wire`, where the same call in-process — the invariant PLACEMENT.md states — just
    // answers.  Parking it in the arena removes the ceiling instead of raising it: the
    // arena grows and its new size already travels back in `ret_words`, and under
    // `remote` its bytes travel with the answer.
    //
    // No new wire tag: the handle rides the existing `TAG_REF`, and the CALLER knows
    // to read a text out of it because the function's return kind says `Text`
    // (`dispatch::finish_call`).  A text that fits keeps the inline path, so the
    // common small answer costs nothing.
    if let crate::host::Value::Text(s) = &out
        && !fits_in_frame(s)
    {
        let anchor = crate::keys::DbRef {
            store_nr: ret_nr,
            rec: 0,
            pos: 0,
        };
        let handle = program.stores().store_mut(&anchor).set_str(s);
        return Ok(crate::host::Value::Ref(crate::keys::DbRef {
            store_nr: ret_nr,
            rec: handle,
            pos: TEXT_IN_ARENA,
        }));
    }
    let crate::host::Value::Ref(answer) = out else {
        return Ok(out);
    };
    if answer.is_null() || answer.store_nr == ret_nr {
        return Ok(crate::host::Value::Ref(answer));
    }
    // The callee ignored the offered destination and built its answer in a store
    // of its own — the ordinary shape for `Point { … }`, where the constructor
    // mints a store and hands ownership to its caller. Copy it into the arena,
    // then free it exactly as an in-process caller's `OpFreeRef` would: this
    // process IS that caller, and nothing else will ever free it.
    //
    // Only when the answer is genuinely OWNED, which is the analysis's verdict
    // and not a second opinion derived here (@PLN119 arc C). A `View` return
    // borrows storage the callee did not create, and freeing that would pull the
    // ground out from under the library's own state — but a `View` return is
    // refused at marking, so reaching this with one means the two sides of the
    // decision have drifted apart.
    let (tp, _) = program.layout_of(&ret_ty);
    let landed = super::arena::alloc_value(program.stores(), ret_nr, tp);
    super::arena::copy_value(program.stores(), &landed, &answer, tp);
    if program.return_delivery(&func) == crate::use_analysis::HeapDelivery::Owned {
        program.stores().free_named(&answer, "<placed return>");
    }
    Ok(crate::host::Value::Ref(landed))
}

// ── remote server ───────────────────────────────────────────────────────────

/// @PLN119 arc E — serve one library's calls over a socket, so a consumer can
/// place it on another machine.
///
/// Entered from `loft --lib-server <addr> <pkg_dir> --default <stdlib>`, the
/// symmetric twin of `--lib-worker`. Never returns.
///
/// # What this is, and what it is not
///
/// It serves **exactly the library named on its command line**, and the protocol
/// carries a function name that is resolved only within that library — there is
/// no path by which a caller reaches anything else. The address is the
/// operator's to choose and there is no default: a service that bound something
/// helpful on its own would be a service nobody decided to run.
///
/// It is **not** an authenticated or encrypted channel, and it is not a sandbox.
/// It executes its library's functions for whoever connects, which is the same
/// trust an in-process `use` already extends — but over a socket that trust has
/// to be arranged by the deployment (a loopback bind, a private network, a
/// tunnel), not assumed. Binding it to a public interface publishes the library.
///
/// # Panics
/// Never deliberately; a fault inside the library is caught and returned to the
/// caller as an error, which is what makes a placed library's error behaviour
/// match an in-process one.
pub fn serve_remote(address: &str, pkg_dir: &Path, stdlib_dir: &Path) -> ! {
    let listener = match std::net::TcpListener::bind(address) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("loft: cannot serve {} on {address}: {e}", pkg_dir.display());
            std::process::exit(1);
        }
    };
    let mut program = match crate::host::Program::from_library_dir(pkg_dir, stdlib_dir) {
        Ok(mut p) => {
            // The same anchoring a local worker gets: a relative path means what
            // it means to the code being served. On another machine that is this
            // server's own directory, which is the only answer available and the
            // one an operator can arrange for.
            if let Ok(here) = std::env::current_dir() {
                p.anchor_paths_at(&here);
            }
            p
        }
        Err(e) => {
            eprintln!("loft: cannot load {} — {e}", pkg_dir.display());
            std::process::exit(1);
        }
    };
    // Scratch of this server's own; the caller's arena bytes land in it.
    let base = std::env::temp_dir().join(format!("loft-serve-{}.wire", std::process::id()));
    let (arg_path, ret_path) = arena_paths(&base);
    let mut arenas = match (Arena::create(&arg_path), Arena::create(&ret_path)) {
        (Ok(a), Ok(r)) => (a, r),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("loft: cannot create the call arena: {e}");
            std::process::exit(1);
        }
    };
    let served = listener
        .local_addr()
        .map_or_else(|_| address.to_string(), |a| a.to_string());
    println!("loft: serving {} on {served}", pkg_dir.display());

    // One caller at a time, which is not a limitation this transport invents:
    // the local wire has a single request slot for the same reason, and a placed
    // library's calls serialise either way.
    loop {
        let Ok(stream) = crate::net_profile::time("placement/accept", None, || {
            listener.accept().map(|(s, _)| s)
        }) else {
            continue;
        };
        let _ = stream.set_nodelay(true);
        serve_connection(stream, &mut program, &mut arenas);
    }
}

/// Serve one connection until the caller goes away.
fn serve_connection(
    mut stream: std::net::TcpStream,
    program: &mut crate::host::Program,
    arenas: &mut (Arena, Arena),
) {
    loop {
        let Ok(parts) = get_message(&mut stream, 2) else {
            return; // the caller closed, or spoke nonsense; either way it is over
        };
        let mut parts = parts.into_iter();
        let mut body = parts.next().unwrap_or_default();
        arenas.0.load_image(&parts.next().unwrap_or_default());
        let mut f = Frame::over(&mut body);
        let reply = match f.get_u32() {
            Some(REQ_SHUTDOWN) | None => return,
            Some(REQ_LAYOUT) => answer_layout(&mut f, Some(program)),
            _ => {
                // The size word a local caller sends is meaningless here — the
                // bytes came with the request — so it is read and dropped, which
                // keeps ONE request shape across both transports.
                f.get_u32();
                serve_bound(&mut f, program, arenas)
            }
        };
        let mut out = vec![0u8; PAYLOAD_BYTES];
        let mut o = Frame::over(&mut out);
        let (ret_words, arg_words) = (arenas.1.words(), arenas.0.words());
        let built = match &reply {
            Ok(v) => {
                o.put_u32(RESP_OK) && o.put_u32(ret_words) && o.put_u32(arg_words) && o.put_value(v)
            }
            Err(e) => {
                o.put_u32(RESP_ERR)
                    && o.put_u32(ret_words)
                    && o.put_u32(arg_words)
                    && o.put_fault(e)
            }
        };
        let used = o.len();
        if !built {
            let mut o = Frame::over(&mut out);
            let _ = o.put_u32(RESP_ERR)
                && o.put_u32(0)
                && o.put_u32(0)
                && o.put_fault(&Fault::from("answer does not fit the placement frame"));
        }
        out.truncate(used.max(16));
        // Both arenas travel home: the return carries the answer, and the
        // ARGUMENT carries whatever the callee wrote into a parameter — which
        // loft's by-reference semantics make the caller's to see.
        if put_message(&mut stream, &[&out, arenas.1.image(), arenas.0.image()]).is_err() {
            return;
        }
    }
}
