// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)

//! The handle a loft `File` keeps open: an OS file with a read buffer in front of it.
//!
//! A binary read (`f#read(2) as i16`) asks for a few bytes at a time, and each used to be
//! its own `read(2)` system call — a 150 000-element vector read cost 150 000 of them.
//! The buffer serves reads from memory and refills in large blocks.
//!
//! The buffer is never observable: every operation behaves as it did on the bare file.
//! The file's OS position runs AHEAD of the program's by the unread bytes the buffer
//! holds, so everything that is not a read first returns the file to the program's
//! (logical) position and drops the buffer — a write lands where the program's position
//! is, a seek relative to the current position is relative to the logical one, and
//! `stream_position` answers the logical one.  A read fills the whole request unless the
//! file ends, exactly as a read of a regular file does, so a caller that asks once for
//! `n` bytes still gets `n`.
//!
//! What a buffer cannot promise is a byte that another handle changes AFTER this one has
//! buffered it; the next refill, write or seek sees the change.
//!
//! A write goes to the file at once, but the loft runtime positions the handle before every
//! value it writes (`f#next`), and that seek is almost always to where the file already is.
//! The handle therefore remembers its logical position after every operation that settles
//! it, and a seek to exactly that position answers without asking the OS.  An error forgets
//! the position, so the next seek goes to the OS again.
//!
//! A handle a READ opened is read-only, so that a program which only reads never opens a
//! file for writing (a watcher sees a write-close for that, and a read-only file could not be
//! read at all).  It keeps its path, and the first write or resize through it reopens the
//! file read-write at the logical position — the program's `File` is one handle whichever
//! operation came first (loft#1861: a write after a read failed with `Bad file descriptor`).

use crate::file_access::PathText;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

/// Bytes a refill asks for.  A request at least this large bypasses the buffer.
const CAPACITY: usize = 64 * 1024;

pub struct LoftFile {
    file: File,
    /// The logical position, when every operation since the last seek has settled it.
    at: Option<u64>,
    buf: Vec<u8>,
    /// The next unread byte in `buf`.
    pos: usize,
    /// The bytes `buf` holds.
    len: usize,
    /// A read-only handle's path, kept to reopen it read-write on its first write.
    reopen: Option<PathText>,
}

impl LoftFile {
    /// A handle opened for writing (and reading).
    #[must_use]
    pub fn new(file: File) -> LoftFile {
        LoftFile {
            file,
            at: None,
            buf: Vec::new(),
            pos: 0,
            len: 0,
            reopen: None,
        }
    }

    /// A handle opened read-only from `path` — a path the runtime resolved, in the program's
    /// spelling (`file_access::at`); a write through it reopens `path` read-write.
    #[must_use]
    pub fn reader(file: File, path: &str) -> LoftFile {
        LoftFile {
            reopen: Some(crate::file_access::at(path)),
            ..LoftFile::new(file)
        }
    }

    /// Before a write: a read-only handle becomes a read-write one at the same logical
    /// position.  A file that cannot be opened for writing answers the OS error, and the
    /// handle stays a reader.
    fn writable(&mut self) -> io::Result<()> {
        let Some(path) = self.reopen.clone() else {
            return Ok(());
        };
        self.realign()?;
        let at = self.file.stream_position()?;
        let mut file = crate::file_access::open_read_write(&path)?;
        file.seek(SeekFrom::Start(at))?;
        self.file = file;
        self.at = Some(at);
        self.reopen = None;
        Ok(())
    }

    /// Unread buffered bytes: how far the OS position runs ahead of the logical one.
    fn ahead(&self) -> i64 {
        (self.len - self.pos) as i64
    }

    /// Return the OS position to the logical one and drop the buffer, before anything
    /// that is not a read touches the file.
    fn realign(&mut self) -> io::Result<()> {
        let ahead = self.ahead();
        self.pos = 0;
        self.len = 0;
        if ahead > 0 {
            self.file.seek(SeekFrom::Current(-ahead))?;
        }
        Ok(())
    }

    /// `File::sync_data` — only WRITES reach the OS, and they are never buffered.
    ///
    /// # Errors
    /// The OS error of the sync.
    pub fn sync_data(&self) -> io::Result<()> {
        self.file.sync_data()
    }

    /// `File::metadata` — the file's size and kind, which a read buffer does not change.
    ///
    /// # Errors
    /// The OS error of the lookup.
    pub fn metadata(&self) -> io::Result<std::fs::Metadata> {
        self.file.metadata()
    }

    /// `File::set_len`, at the logical position.
    ///
    /// # Errors
    /// The OS error of the realigning seek or of the resize.
    pub fn set_len(&mut self, size: u64) -> io::Result<()> {
        self.writable()?;
        self.realign()?;
        self.file.set_len(size)
    }

    /// Advance the remembered position by `n` bytes, or forget it on an error.
    fn settle<T>(&mut self, r: io::Result<T>, n: impl Fn(&T) -> u64) -> io::Result<T> {
        match &r {
            Ok(v) => self.at = self.at.map(|a| a + n(v)),
            Err(_) => self.at = None,
        }
        r
    }
}

impl Read for LoftFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.take_buffered(out) {
            return Ok(out.len());
        }
        let r = self.fill(out);
        self.settle(r, |n| *n as u64)
    }

    /// `fill` already reads the whole request unless the file ends, so one `read` answers
    /// it — and a request the buffer holds is one copy (a binary format's scalar per call).
    fn read_exact(&mut self, out: &mut [u8]) -> io::Result<()> {
        if self.take_buffered(out) {
            return Ok(());
        }
        if self.read(out)? == out.len() {
            Ok(())
        } else {
            Err(io::Error::from(io::ErrorKind::UnexpectedEof))
        }
    }
}

impl LoftFile {
    /// A request the buffer already holds whole: copied, and the logical position moved by
    /// it — exactly what `fill` and `settle` do for it, without their loop.  `false` leaves
    /// everything as it was.
    #[inline]
    fn take_buffered(&mut self, out: &mut [u8]) -> bool {
        let n = out.len();
        if n == 0 || self.len - self.pos < n {
            return false;
        }
        out.copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        self.at = self.at.map(|a| a + n as u64);
        true
    }

    /// The read itself: the whole request, from the buffer and the file, unless the file ends.
    fn fill(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut done = 0;
        while done < out.len() {
            if self.pos < self.len {
                let n = (self.len - self.pos).min(out.len() - done);
                out[done..done + n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
                self.pos += n;
                done += n;
                continue;
            }
            let rest = out.len() - done;
            if rest >= CAPACITY {
                let n = self.file.read(&mut out[done..])?;
                if n == 0 {
                    break;
                }
                done += n;
                continue;
            }
            if self.buf.len() < CAPACITY {
                self.buf.resize(CAPACITY, 0);
            }
            let n = self.file.read(&mut self.buf)?;
            self.pos = 0;
            self.len = n;
            if n == 0 {
                break;
            }
        }
        Ok(done)
    }
}

impl Write for LoftFile {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if let Err(e) = self.writable().and_then(|()| self.realign()) {
            self.at = None;
            return Err(e);
        }
        let r = self.file.write(data);
        self.settle(r, |n| *n as u64)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for LoftFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        // Already there: nothing moves, the buffer stays valid, the OS is not asked.
        if let (SeekFrom::Start(x), Some(a)) = (to, self.at)
            && x == a
        {
            return Ok(a);
        }
        let to = match to {
            SeekFrom::Current(d) => SeekFrom::Current(d - self.ahead()),
            other => other,
        };
        self.pos = 0;
        self.len = 0;
        let r = self.file.seek(to);
        self.at = r.as_ref().ok().copied();
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(p: &std::path::Path) -> crate::file_access::PathText {
        crate::file_access::PathText::from_os(p)
    }

    fn scratch(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("loft_file_{name}_{}", std::process::id()));
        crate::file_access::write(host(&p), bytes).unwrap();
        p
    }

    fn open(p: &std::path::Path) -> LoftFile {
        LoftFile::new(crate::file_access::open_read_write(host(p)).unwrap())
    }

    #[test]
    fn small_reads_see_every_byte_in_order() {
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let p = scratch("small", &data);
        let mut f = open(&p);
        let mut got = Vec::new();
        let mut two = [0u8; 2];
        while f.read(&mut two).unwrap() == 2 {
            got.extend_from_slice(&two);
        }
        assert_eq!(got, data);
        crate::file_access::remove_file(host(&p)).unwrap();
    }

    #[test]
    fn a_read_fills_the_request_across_a_refill_and_stops_at_the_end() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 7) as u8).collect();
        let path = scratch("fill", &data);
        let mut file = open(&path);
        let mut first = vec![0u8; CAPACITY - 3];
        assert_eq!(file.read(&mut first).unwrap(), first.len());
        let mut ten = vec![0u8; 10];
        assert_eq!(file.read(&mut ten).unwrap(), 10);
        assert_eq!(&ten[..], &data[CAPACITY - 3..CAPACITY + 7]);
        let mut rest = vec![0u8; 200_000];
        let got = file.read(&mut rest).unwrap();
        assert_eq!(got, data.len() - (CAPACITY + 7));
        assert_eq!(file.read(&mut ten).unwrap(), 0);
        crate::file_access::remove_file(host(&path)).unwrap();
    }

    #[test]
    fn a_write_after_a_read_lands_at_the_logical_position() {
        let p = scratch("write", b"abcdefghij");
        let mut f = open(&p);
        let mut three = [0u8; 3];
        f.read_exact(&mut three).unwrap();
        f.write_all(b"XY").unwrap();
        let mut rest = [0u8; 5];
        f.read_exact(&mut rest).unwrap();
        assert_eq!(&rest, b"fghij");
        drop(f);
        assert_eq!(crate::file_access::read(host(&p)).unwrap(), b"abcXYfghij");
        crate::file_access::remove_file(host(&p)).unwrap();
    }

    #[test]
    fn a_reader_written_through_reopens_at_the_logical_position() {
        let p = scratch("reader", b"abcdefghij");
        let mut f = LoftFile::reader(
            crate::file_access::open(host(&p)).unwrap(),
            &p.to_string_lossy(),
        );
        let mut three = [0u8; 3];
        f.read_exact(&mut three).unwrap();
        f.write_all(b"XY").unwrap();
        let mut rest = [0u8; 5];
        f.read_exact(&mut rest).unwrap();
        assert_eq!(&rest, b"fghij");
        drop(f);
        assert_eq!(crate::file_access::read(host(&p)).unwrap(), b"abcXYfghij");
        crate::file_access::remove_file(host(&p)).unwrap();
    }

    #[test]
    fn a_seek_to_the_remembered_position_keeps_every_write_in_place() {
        let p = scratch("cached", b"");
        let mut f = open(&p);
        assert_eq!(f.seek(SeekFrom::Start(0)).unwrap(), 0);
        for (i, b) in [b"ab", b"cd", b"ef"].iter().enumerate() {
            // the runtime's shape: position, then write — the position is where the file is
            assert_eq!(f.seek(SeekFrom::Start(2 * i as u64)).unwrap(), 2 * i as u64);
            f.write_all(*b).unwrap();
        }
        assert_eq!(f.seek(SeekFrom::Start(1)).unwrap(), 1);
        let mut two = [0u8; 2];
        f.read_exact(&mut two).unwrap();
        assert_eq!(&two, b"bc");
        // Buffered bytes are unread, so the OS runs ahead: the cached 3 must still land at 3.
        assert_eq!(f.seek(SeekFrom::Start(3)).unwrap(), 3);
        f.write_all(b"Z").unwrap();
        assert_eq!(f.stream_position().unwrap(), 4);
        drop(f);
        assert_eq!(crate::file_access::read(host(&p)).unwrap(), b"abcZef");
        crate::file_access::remove_file(host(&p)).unwrap();
    }

    #[test]
    fn seeks_answer_and_move_from_the_logical_position() {
        let p = scratch("seek", b"0123456789");
        let mut f = open(&p);
        let mut two = [0u8; 2];
        f.read_exact(&mut two).unwrap();
        assert_eq!(f.stream_position().unwrap(), 2);
        assert_eq!(f.seek(SeekFrom::Current(3)).unwrap(), 5);
        f.read_exact(&mut two).unwrap();
        assert_eq!(&two, b"56");
        assert_eq!(f.seek(SeekFrom::End(-1)).unwrap(), 9);
        f.read_exact(&mut [0u8; 1]).unwrap();
        assert_eq!(f.seek(SeekFrom::Start(1)).unwrap(), 1);
        f.read_exact(&mut two).unwrap();
        assert_eq!(&two, b"12");
        crate::file_access::remove_file(host(&p)).unwrap();
    }
}
