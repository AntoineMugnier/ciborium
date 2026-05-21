// SPDX-License-Identifier: Apache-2.0

use super::*;

use ciborium_io::Read;
#[cfg(any(feature = "alloc", feature = "std"))]
use ciborium_io::ReadRc;
#[cfg(any(feature = "alloc", feature = "std"))]
pub use ciborium_io::RcVecSlice;

/// An error that occurred while decoding
#[derive(Clone, Debug)]
pub enum Error<T> {
    /// An error occurred while reading bytes
    ///
    /// Contains the underlying error returned while reading.
    Io(T),

    /// An error occurred while parsing bytes
    ///
    /// Contains the offset into the stream where the syntax error occurred.
    Syntax(usize),
}

impl<T> From<T> for Error<T> {
    #[inline]
    fn from(value: T) -> Self {
        Self::Io(value)
    }
}

/// A decoder for deserializing CBOR items
///
/// Tracks the byte offset and a one-item push-back buffer. The reader is
/// passed explicitly to each method so the decoder's lifetime is independent
/// of any particular reader.
pub struct Decoder {
    offset: usize,
    buffer: Option<Title>,
}

impl Default for Decoder {
    #[inline]
    fn default() -> Self {
        Self {
            offset: 0,
            buffer: None,
        }
    }
}

impl Decoder {
    /// Reads exactly `data.len()` bytes from the reader, advancing the offset.
    #[inline]
    pub fn read_exact<R: Read>(
        &mut self,
        reader: &mut R,
        data: &mut [u8],
    ) -> Result<(), R::Error> {
        assert!(self.buffer.is_none());
        reader.read_exact(data)?;
        self.offset += data.len();
        Ok(())
    }

    #[inline]
    fn pull_title<R: Read>(&mut self, reader: &mut R) -> Result<Title, Error<R::Error>> {
        if let Some(title) = self.buffer.take() {
            self.offset += title.1.as_ref().len() + 1;
            return Ok(title);
        }

        let mut prefix = [0u8; 1];
        self.read_exact(reader, &mut prefix[..])?;

        let major = match prefix[0] >> 5 {
            0 => Major::Positive,
            1 => Major::Negative,
            2 => Major::Bytes,
            3 => Major::Text,
            4 => Major::Array,
            5 => Major::Map,
            6 => Major::Tag,
            7 => Major::Other,
            _ => unreachable!(),
        };

        let mut minor = match prefix[0] & 0b00011111 {
            x if x < 24 => Minor::This(x),
            24 => Minor::Next1([0; 1]),
            25 => Minor::Next2([0; 2]),
            26 => Minor::Next4([0; 4]),
            27 => Minor::Next8([0; 8]),
            31 => Minor::More,
            _ => return Err(Error::Syntax(self.offset - 1)),
        };

        self.read_exact(reader, minor.as_mut())?;
        Ok(Title(major, minor))
    }

    #[inline]
    fn push_title(&mut self, item: Title) {
        assert!(self.buffer.is_none());
        // The bytes for this title have already been consumed from the reader
        // and counted in self.offset. We subtract them back so that offset()
        // reports the position before this title.
        self.offset -= item.1.as_ref().len() + 1;
        self.buffer = Some(item);
    }

    /// Pulls the next header from the input
    #[inline]
    pub fn pull<R: Read>(&mut self, reader: &mut R) -> Result<Header, Error<R::Error>> {
        let offset = self.offset;
        self.pull_title(reader)?
            .try_into()
            .map_err(|_| Error::Syntax(offset))
    }

    /// Push a single header into the input buffer
    ///
    /// # Panics
    ///
    /// This function panics if called while there is already a header in the
    /// input buffer. You should take care to call this function only after
    /// pulling a header to ensure there is nothing in the input buffer.
    #[inline]
    pub fn push(&mut self, item: Header) {
        self.push_title(Title::from(item))
    }

    /// Reads the next `len` bytes as a reference-counted slice.
    ///
    /// Delegates to `ReadRc::read_rc` so no copying occurs when the reader
    /// already owns an `Rc<Vec<u8>>` (e.g. `RcVecBuf`). For readers that must
    /// allocate (e.g. `&[u8]`), a single `Vec` allocation is made.
    #[cfg(any(feature = "alloc", feature = "std"))]
    #[inline]
    pub fn read_exact_rc<R: ReadRc>(
        &mut self,
        reader: &mut R,
        len: usize,
    ) -> Result<RcVecSlice, Error<R::Error>> {
        assert!(self.buffer.is_none());
        let slice = reader.read_rc(len).map_err(Error::Io)?;
        self.offset += len;
        Ok(slice)
    }

    /// Gets the current byte offset into the stream
    #[inline]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Process an incoming bytes item
    #[inline]
    pub fn bytes<'a, R: Read>(
        &'a mut self,
        reader: &'a mut R,
        len: Option<usize>,
    ) -> Segments<'a, R, crate::seg::Bytes> {
        self.push(Header::Bytes(len));
        Segments::new(self, reader, |header| match header {
            Header::Bytes(len) => Ok(len),
            _ => Err(()),
        })
    }

    /// Process an incoming text item
    #[inline]
    pub fn text<'a, R: Read>(
        &'a mut self,
        reader: &'a mut R,
        len: Option<usize>,
    ) -> Segments<'a, R, crate::seg::Text> {
        self.push(Header::Text(len));
        Segments::new(self, reader, |header| match header {
            Header::Text(len) => Ok(len),
            _ => Err(()),
        })
    }
}
