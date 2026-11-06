// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I69 — Word-addressed store: the file a file-backed store maps

//! A file mapped read-write into memory: what a file-backed store (`Store::open`) grows and
//! reads through.
//!
//! It is the four calls the store makes on `mmap-storage` 0.10's `file::Storage`, with the same
//! behaviour, over `memmap2`: `memmap`, which that crate is built on, is unmaintained
//! (its advisory is in THIRDPARTY.md).  Two of those behaviours the store relies on:
//!
//! - `as_slice` covers the bytes WRITTEN so far (the file's length at `open`, then the last
//!   `resize`), so a new, empty file gives an empty slice until it is resized.
//! - A `resize` whose remap fails leaves NO mapping, and the next `as_slice` panics.  The store
//!   re-derives its pointer right after a failed resize so that this surfaces there, never as a
//!   read through a dangling pointer.

use crate::file_access::{self as fa, File};
use std::io;

pub struct MmapFile {
    /// `None` only between unmapping and remapping in `resize`, or after a remap failed.
    inner: Option<memmap2::MmapMut>,
    file: File,
    /// The end of the written data: what `as_slice` covers.
    end: usize,
    /// The file's size: what `capacity` answers.
    size: usize,
}

impl MmapFile {
    /// Open the file at `path` for reading and writing, creating it when absent.  An empty file
    /// is given one byte, since an empty file cannot be mapped.
    ///
    /// # Errors
    /// When the file cannot be opened, sized or mapped.
    pub fn open(path: &str) -> io::Result<Self> {
        let file = fa::open_or_create_read_write(path)?;
        let len = usize::try_from(fa::metadata(path)?.len()).map_err(io::Error::other)?;
        let mut size = len;
        if size == 0 {
            file.set_len(1)?;
            size = 1;
        }
        let mut mapped = Self {
            inner: None,
            file,
            end: len,
            size,
        };
        mapped.map()?;
        Ok(mapped)
    }

    fn map(&mut self) -> io::Result<()> {
        // SAFETY: the store owns this file for as long as the mapping lives; a writer changing
        // it from outside is the hazard every shared mapping has.
        self.inner = Some(unsafe { memmap2::MmapMut::map_mut(&self.file)? });
        Ok(())
    }

    /// The written bytes.
    ///
    /// # Panics
    /// After a `resize` whose remap failed: there is no mapping left to read.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        let map = self
            .inner
            .as_ref()
            .expect("the store's file mapping is gone: a resize could not remap it");
        &map[..self.end]
    }

    /// The file's size in bytes.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.size
    }

    /// Grow or shrink the file to `new_size` bytes (at least one) and map it again.  When the
    /// file cannot be resized, the old mapping is restored and the error returned.
    ///
    /// # Errors
    /// When the file cannot be resized, or cannot be mapped again.
    pub fn resize(&mut self, new_size: usize) -> io::Result<()> {
        self.inner = None;
        let file_size = if new_size == 0 { 1 } else { new_size as u64 };
        if let Err(error) = self.file.set_len(file_size) {
            self.map()?;
            return Err(error);
        }
        self.size = usize::try_from(file_size).map_err(io::Error::other)?;
        self.end = new_size;
        self.map()
    }
}
