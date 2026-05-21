// SPDX-License-Identifier: Apache-2.0

//! Simple, Low-level I/O traits
//!
//! This crate provides two simple traits: `Read` and `Write`. These traits
//! mimic their counterparts in `std::io`, but are trimmed for simplicity
//! and can be used in `no_std` and `no_alloc` environments. Since this
//! crate contains only traits, inline functions and unit structs, it should
//! be a zero-cost abstraction.
//!
//! If the `std` feature is enabled, we provide blanket implementations for
//! all `std::io` types. If the `alloc` feature is enabled, we provide
//! implementations for `Vec<u8>`. In all cases, you get implementations
//! for byte slices. You can, of course, implement the traits for your own
//! types.

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(clippy::cargo)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
use std::{rc::Rc, vec::Vec};

#[cfg(all(feature = "alloc", not(feature = "std")))]
use alloc::{rc::Rc, vec::Vec};

/// Adapters of embedded-io::{Read, Write} implementing ciborium::{Read, Write}
#[cfg(feature = "embedded-io")]
pub mod eio;

/// A trait indicating a type that can read bytes
///
/// Note that this is similar to `std::io::Read`, but simplified for use in a
/// `no_std` context.
pub trait Read {
    /// The error type
    type Error;

    /// Reads exactly `data.len()` bytes or fails
    fn read_exact(&mut self, data: &mut [u8]) -> Result<(), Self::Error>;
}

/// A zero-copy read trait that returns a reference-counted slice
///
/// Implementors can hand back a view into an existing `Rc<Vec<u8>>` without
/// copying any bytes.
#[cfg(any(feature = "alloc", feature = "std"))]
pub trait ReadRc: Read {
    /// Reads `len` bytes and returns them as an `RcVecSlice`
    fn read_rc(&mut self, len: usize) -> Result<RcVecSlice, Self::Error>;
}

/// A zero-copy read trait that borrows directly from the input
///
/// Only implementable for in-memory readers whose backing storage outlives the
/// borrow (e.g. `&'de [u8]`).
pub trait BorrowRead: Read {
    /// Returns a reference to the next `len` bytes, advancing past them.
    fn borrow_read(&mut self, len: usize) -> Result<&[u8], Self::Error>;
}

/// A view into a reference-counted byte buffer
#[cfg(any(feature = "alloc", feature = "std"))]
pub struct RcVecSlice {
    /// The underlying buffer
    pub buf: Rc<Vec<u8>>,
    /// The start index of this slice within `buf`
    pub start_index: usize,
    /// The length of this slice
    pub len: usize,
}

impl Read for &[u8] {
    type Error = EndOfFile;

    #[inline]
    fn read_exact(&mut self, data: &mut [u8]) -> Result<(), Self::Error> {
        if data.len() > self.len() {
            return Err(EndOfFile(()));
        }
        let (prefix, suffix) = self.split_at(data.len());
        data.copy_from_slice(prefix);
        *self = suffix;
        Ok(())
    }
}

#[cfg(any(feature = "alloc", feature = "std"))]
impl ReadRc for &[u8] {
    #[inline]
    fn read_rc(&mut self, len: usize) -> Result<RcVecSlice, Self::Error> {
        if len > self.len() {
            return Err(EndOfFile(()));
        }
        let (prefix, suffix) = self.split_at(len);
        *self = suffix;
        let buf = Rc::new(prefix.to_vec());
        Ok(RcVecSlice { buf, start_index: 0, len })
    }
}

impl BorrowRead for &[u8] {
    #[inline]
    fn borrow_read(&mut self, len: usize) -> Result<&[u8], Self::Error> {
        if len > self.len() {
            return Err(EndOfFile(()));
        }
        let (prefix, suffix) = self.split_at(len);
        *self = suffix;
        Ok(prefix)
    }
}

impl<R: Read + ?Sized> Read for &mut R {
    type Error = R::Error;

    #[inline]
    fn read_exact(&mut self, data: &mut [u8]) -> Result<(), Self::Error> {
        (**self).read_exact(data)
    }
}

#[cfg(any(feature = "alloc", feature = "std"))]
impl<R: ReadRc + ?Sized> ReadRc for &mut R {
    #[inline]
    fn read_rc(&mut self, len: usize) -> Result<RcVecSlice, Self::Error> {
        (**self).read_rc(len)
    }
}

impl<R: BorrowRead + ?Sized> BorrowRead for &mut R {
    #[inline]
    fn borrow_read(&mut self, len: usize) -> Result<&[u8], Self::Error> {
        (**self).borrow_read(len)
    }
}

/// A buffered reader backed by a reference-counted byte vector
#[cfg(any(feature = "alloc", feature = "std"))]
pub struct RcVecBuf {
    /// The underlying buffer
    pub rc: Rc<Vec<u8>>,
    /// Current read position
    pub cursor: usize,
}

#[cfg(any(feature = "alloc", feature = "std"))]
impl Read for RcVecBuf {
    type Error = EndOfFile;

    #[inline]
    fn read_exact(&mut self, data: &mut [u8]) -> Result<(), Self::Error> {
        let end = self.cursor + data.len();
        if end > self.rc.len() {
            return Err(EndOfFile(()));
        }
        data.copy_from_slice(&self.rc[self.cursor..end]);
        self.cursor = end;
        Ok(())
    }
}

#[cfg(any(feature = "alloc", feature = "std"))]
impl ReadRc for RcVecBuf {
    #[inline]
    fn read_rc(&mut self, len: usize) -> Result<RcVecSlice, Self::Error> {
        let end = self.cursor + len;
        if end > self.rc.len() {
            return Err(EndOfFile(()));
        }
        let slice = RcVecSlice {
            buf: self.rc.clone(),
            start_index: self.cursor,
            len,
        };
        self.cursor = end;
        Ok(slice)
    }
}

/// A trait indicating a type that can add byte slices to its buffer
pub trait WriteByteSlice<'a>: Write {
    /// Add a byte slice to the Writer
    fn add_slice(&mut self, data: &'a [u8]) -> Result<(), Self::Error>;
}

/// A trait indicating a type that can write bytes
///
/// Note that this is similar to `std::io::Write`, but simplified for use in a
/// `no_std` context.
pub trait Write {
    /// The error type
    type Error;

    /// Writes all bytes from `data` or fails
    fn write_all(&mut self, data: &[u8]) -> Result<(), Self::Error>;

    /// Flushes all output
    fn flush(&mut self) -> Result<(), Self::Error>;
}

#[cfg(feature = "std")]
impl<T: std::io::Write> Write for T {
    type Error = std::io::Error;

    #[inline]
    fn write_all(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.write_all(data)
    }

    #[inline]
    fn flush(&mut self) -> Result<(), Self::Error> {
        self.flush()
    }
}

#[cfg(not(feature = "std"))]
impl<W: Write + ?Sized> Write for &mut W {
    type Error = W::Error;

    #[inline]
    fn write_all(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        (**self).write_all(data)
    }

    #[inline]
    fn flush(&mut self) -> Result<(), Self::Error> {
        (**self).flush()
    }
}

/// An error indicating there are no more bytes to read
#[derive(Clone, Debug)]
pub struct EndOfFile(());

#[cfg(any(feature = "alloc", feature = "std"))]
/// A scatter-gather writer backed by a fixed scratch buffer for encoded bytes and
/// a `Vec` for the accumulated slice references.
pub struct ByteSliceWriter<'a, 'b: 'a> {
    buf: &'b mut [u8],
    slices: Vec<&'a [u8]>,
}

#[cfg(any(feature = "alloc", feature = "std"))]
impl<'a, 'b> ByteSliceWriter<'a, 'b> {
    /// Creates a new `ByteSliceWriter` with the given scratch buffer and an
    /// existing (possibly pre-allocated) slices vector.
    pub fn new(buf: &'b mut [u8], slices: Vec<&'a [u8]>) -> Self {
        Self { buf, slices }
    }

    /// Consumes the writer and returns the accumulated scatter-gather slice list.
    pub fn into_vec(self) -> Vec<&'a [u8]> {
        self.slices
    }
}

#[cfg(any(feature = "alloc", feature = "std"))]
impl<'a, 'b> WriteByteSlice<'a> for ByteSliceWriter<'a, 'b> {
    fn add_slice(&mut self, data: &'a [u8]) -> Result<(), Self::Error> {
        self.slices.push(data);
        Ok(())
    }
}

#[cfg(any(feature = "alloc", feature = "std"))]
impl<'a, 'b> Write for ByteSliceWriter<'a, 'b> {
    type Error = EndOfFile;
    fn write_all(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        if data.len() > self.buf.len() {
            return Err(EndOfFile(()));
        }

        let buf = core::mem::take(&mut self.buf);

        let (pre, suf_buf) = buf.split_at_mut(data.len());

        pre.copy_from_slice(data);

        self.buf = suf_buf;

        self.slices.push(pre);

        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// An error indicating that the output cannot accept more bytes
#[cfg(not(feature = "std"))]
#[derive(Clone, Debug)]
pub struct OutOfSpace(());

#[cfg(not(feature = "std"))]
impl Write for &mut [u8] {
    type Error = OutOfSpace;

    #[inline]
    fn write_all(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        if data.len() > self.len() {
            return Err(OutOfSpace(()));
        }

        let (prefix, suffix) = core::mem::take(self).split_at_mut(data.len());
        prefix.copy_from_slice(data);
        *self = suffix;
        Ok(())
    }

    #[inline]
    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(all(not(feature = "std"), feature = "alloc"))]
impl Write for alloc::vec::Vec<u8> {
    type Error = core::convert::Infallible;

    #[inline]
    fn write_all(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.extend_from_slice(data);
        Ok(())
    }

    #[inline]
    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn read_eof() {
        let mut reader = &[1u8; 0][..];
        let mut buffer = [0u8; 1];

        reader.read_exact(&mut buffer[..]).unwrap_err();
    }

    #[test]
    fn read_one() {
        let mut reader = &[1u8; 1][..];
        let mut buffer = [0u8; 1];

        reader.read_exact(&mut buffer[..]).unwrap();
        assert_eq!(buffer[0], 1);

        reader.read_exact(&mut buffer[..]).unwrap_err();
    }

    #[test]
    fn read_two() {
        let mut reader = &[1u8; 2][..];
        let mut buffer = [0u8; 1];

        reader.read_exact(&mut buffer[..]).unwrap();
        assert_eq!(buffer[0], 1);

        reader.read_exact(&mut buffer[..]).unwrap();
        assert_eq!(buffer[0], 1);

        reader.read_exact(&mut buffer[..]).unwrap_err();
    }

    #[test]
    fn write_oos() {
        let mut writer = &mut [0u8; 0][..];

        writer.write_all(&[1u8; 1][..]).unwrap_err();
    }

    #[test]
    fn write_one() {
        let mut buffer = [0u8; 1];
        let mut writer = &mut buffer[..];

        writer.write_all(&[1u8; 1][..]).unwrap();
        writer.write_all(&[1u8; 1][..]).unwrap_err();
        assert_eq!(buffer[0], 1);
    }

    #[test]
    fn write_two() {
        let mut buffer = [0u8; 2];
        let mut writer = &mut buffer[..];

        writer.write_all(&[1u8; 1][..]).unwrap();
        writer.write_all(&[1u8; 1][..]).unwrap();
        writer.write_all(&[1u8; 1][..]).unwrap_err();
        assert_eq!(buffer[0], 1);
        assert_eq!(buffer[1], 1);
    }

    #[test]
    #[cfg(feature = "alloc")]
    fn write_vec() {
        let mut buffer = alloc::vec::Vec::new();

        buffer.write_all(&[1u8; 1][..]).unwrap();
        buffer.write_all(&[1u8; 1][..]).unwrap();

        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer[0], 1);
        assert_eq!(buffer[1], 1);
    }

    #[test]
    #[cfg(feature = "std")]
    fn write_std() {
        let mut writer = std::io::sink();

        writer.write_all(&[1u8; 1][..]).unwrap();
        writer.write_all(&[1u8; 1][..]).unwrap();
    }

    #[test]
    #[cfg(feature = "alloc")]
    fn borrow_read_slice() {
        let mut reader = &[1u8, 2u8, 3u8][..];
        let chunk = reader.borrow_read(2).unwrap();
        assert_eq!(chunk, &[1u8, 2u8]);
        assert_eq!(reader, &[3u8]);
    }

    #[test]
    #[cfg(feature = "alloc")]
    fn rc_vec_buf_read() {
        use std::rc::Rc;
        let mut buf = RcVecBuf { rc: Rc::new(vec![10u8, 20u8, 30u8]), cursor: 0 };
        let mut data = [0u8; 2];
        buf.read_exact(&mut data).unwrap();
        assert_eq!(data, [10u8, 20u8]);
        assert_eq!(buf.cursor, 2);
        buf.read_exact(&mut data).unwrap_err();
    }

    #[test]
    #[cfg(feature = "alloc")]
    fn rc_vec_buf_read_rc() {
        use std::rc::Rc;
        let mut buf = RcVecBuf { rc: Rc::new(vec![10u8, 20u8, 30u8]), cursor: 0 };
        let slice = buf.read_rc(2).unwrap();
        assert_eq!(buf.cursor, 2);
        assert_eq!(slice.start_index, 0);
        assert_eq!(slice.len, 2);
        assert_eq!(&slice.buf[slice.start_index..slice.start_index + slice.len], &[10u8, 20u8]);
    }
}
