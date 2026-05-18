// SPDX-License-Identifier: Apache-2.0

use alloc::vec::Vec;
use core::marker::PhantomData;

use ciborium_io::{ByteSliceWriter, EndOfFile, Write, WriteByteSlice};
use ciborium_ll::*;
use serde::{ser, Serialize as _};

use super::Error;

/// A zero-copy CBOR serializer that produces a scatter-gather list of byte slices.
///
/// Unlike the standard [`into_writer`](super::into_writer) path, string and bytes
/// values are referenced in-place (zero-copy) via [`WriteByteSlice::add_slice`].
/// Scalar values (integers, floats, booleans, CBOR headers) are encoded into a
/// caller-supplied scratch buffer `buf`.
///
/// The typical entry point is [`into_bytes_rc`], which manages the collector
/// internally and returns a `Vec<&'a [u8]>` scatter-gather list.
///
/// # Safety invariant
///
/// The `Serialize` implementations of any serialized type must only pass byte
/// slices that are borrowed from the value itself (with lifetime `>= 'a`) to
/// `serialize_str` and `serialize_bytes`. Standard `#[derive(Serialize)]`
/// always satisfies this. Hand-written implementations that construct temporaries
/// inside `serialize()` and pass references to them do not.
pub struct ByteSliceSerializer<'a, W>(Encoder<W>, PhantomData<&'a ()>);

impl<'a, W: Write + WriteByteSlice<'a>> From<W> for ByteSliceSerializer<'a, W> {
    #[inline]
    fn from(writer: W) -> Self {
        Self(writer.into(), PhantomData)
    }
}

impl<'a, W: Write + WriteByteSlice<'a>> From<Encoder<W>> for ByteSliceSerializer<'a, W> {
    #[inline]
    fn from(encoder: Encoder<W>) -> Self {
        Self(encoder, PhantomData)
    }
}

impl<'a, W: Write + WriteByteSlice<'a>> ByteSliceSerializer<'a, W> {
    /// Consumes the serializer and returns the inner writer.
    #[inline]
    pub fn into_inner(self) -> W {
        self.0.into_inner()
    }
}

impl<'a, 'b, W: Write + WriteByteSlice<'a>> ser::Serializer for &'b mut ByteSliceSerializer<'a, W>
where
    W::Error: core::fmt::Debug,
{
    type Ok = ();
    type Error = Error<W::Error>;

    type SerializeSeq = ZeroCopyCollectionSerializer<'a, 'b, W>;
    type SerializeTuple = ZeroCopyCollectionSerializer<'a, 'b, W>;
    type SerializeTupleStruct = ZeroCopyCollectionSerializer<'a, 'b, W>;
    type SerializeTupleVariant = ZeroCopyCollectionSerializer<'a, 'b, W>;
    type SerializeMap = ZeroCopyCollectionSerializer<'a, 'b, W>;
    type SerializeStruct = ZeroCopyCollectionSerializer<'a, 'b, W>;
    type SerializeStructVariant = ZeroCopyCollectionSerializer<'a, 'b, W>;

    #[inline]
    fn serialize_bool(self, v: bool) -> Result<(), Self::Error> {
        Ok(self.0.push(match v {
            false => Header::Simple(simple::FALSE),
            true => Header::Simple(simple::TRUE),
        })?)
    }

    #[inline]
    fn serialize_i8(self, v: i8) -> Result<(), Self::Error> {
        self.serialize_i64(v.into())
    }

    #[inline]
    fn serialize_i16(self, v: i16) -> Result<(), Self::Error> {
        self.serialize_i64(v.into())
    }

    #[inline]
    fn serialize_i32(self, v: i32) -> Result<(), Self::Error> {
        self.serialize_i64(v.into())
    }

    #[inline]
    fn serialize_i64(self, v: i64) -> Result<(), Self::Error> {
        Ok(self.0.push(match v.is_negative() {
            false => Header::Positive(v as u64),
            true => Header::Negative(v as u64 ^ !0),
        })?)
    }

    #[inline]
    fn serialize_i128(self, v: i128) -> Result<(), Self::Error> {
        let (tag, raw) = match v.is_negative() {
            false => (tag::BIGPOS, v as u128),
            true => (tag::BIGNEG, v as u128 ^ !0),
        };

        match (tag, u64::try_from(raw)) {
            (tag::BIGPOS, Ok(x)) => return Ok(self.0.push(Header::Positive(x))?),
            (tag::BIGNEG, Ok(x)) => return Ok(self.0.push(Header::Negative(x))?),
            _ => {}
        }

        let first_non_zero_byte = raw.leading_zeros() as usize / 8;
        let slice = &raw.to_be_bytes()[first_non_zero_byte..];

        self.0.push(Header::Tag(tag))?;
        self.0.push(Header::Bytes(Some(slice.len())))?;
        Ok(self.0.write_all(slice)?)
    }

    #[inline]
    fn serialize_u8(self, v: u8) -> Result<(), Self::Error> {
        self.serialize_u64(v.into())
    }

    #[inline]
    fn serialize_u16(self, v: u16) -> Result<(), Self::Error> {
        self.serialize_u64(v.into())
    }

    #[inline]
    fn serialize_u32(self, v: u32) -> Result<(), Self::Error> {
        self.serialize_u64(v.into())
    }

    #[inline]
    fn serialize_u64(self, v: u64) -> Result<(), Self::Error> {
        Ok(self.0.push(Header::Positive(v))?)
    }

    #[inline]
    fn serialize_u128(self, v: u128) -> Result<(), Self::Error> {
        if let Ok(x) = u64::try_from(v) {
            return self.serialize_u64(x);
        }

        let first_non_zero_byte = v.leading_zeros() as usize / 8;
        let slice = &v.to_be_bytes()[first_non_zero_byte..];

        self.0.push(Header::Tag(tag::BIGPOS))?;
        self.0.push(Header::Bytes(Some(slice.len())))?;
        Ok(self.0.write_all(slice)?)
    }

    #[inline]
    fn serialize_f32(self, v: f32) -> Result<(), Self::Error> {
        self.serialize_f64(v.into())
    }

    #[inline]
    fn serialize_f64(self, v: f64) -> Result<(), Self::Error> {
        Ok(self.0.push(Header::Float(v))?)
    }

    #[inline]
    fn serialize_char(self, v: char) -> Result<(), Self::Error> {
        // Encode the char into a stack buffer and write it (copy path, char is small).
        let mut tmp = [0u8; 4];
        let s = v.encode_utf8(&mut tmp);
        let bytes = s.as_bytes();
        self.0.push(Header::Text(Some(bytes.len())))?;
        Ok(self.0.write_all(bytes)?)
    }

    #[inline]
    fn serialize_str(self, v: &str) -> Result<(), Self::Error> {
        let bytes = v.as_bytes();
        self.0.push(Header::Text(Some(bytes.len())))?;
        // SAFETY: `v` is borrowed from the value passed to `into_bytes_rc` (or the
        // equivalent user-constructed `ByteSliceSerializer`), which has lifetime 'a.
        // Standard `#[derive(Serialize)]` only borrows from `self`, so the actual
        // lifetime of `v` is >= 'a. See the safety invariant on `ByteSliceSerializer`.
        let bytes_a: &'a [u8] = unsafe { core::mem::transmute(bytes) };
        Ok(self.0.add_slice(bytes_a)?)
    }

    #[inline]
    fn serialize_bytes(self, v: &[u8]) -> Result<(), Self::Error> {
        self.0.push(Header::Bytes(Some(v.len())))?;
        // SAFETY: same as serialize_str
        let v_a: &'a [u8] = unsafe { core::mem::transmute(v) };
        Ok(self.0.add_slice(v_a)?)
    }

    #[inline]
    fn serialize_none(self) -> Result<(), Self::Error> {
        Ok(self.0.push(Header::Simple(simple::NULL))?)
    }

    #[inline]
    fn serialize_some<U: ?Sized + ser::Serialize>(self, value: &U) -> Result<(), Self::Error> {
        value.serialize(self)
    }

    #[inline]
    fn serialize_unit(self) -> Result<(), Self::Error> {
        self.serialize_none()
    }

    #[inline]
    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), Self::Error> {
        self.serialize_unit()
    }

    #[inline]
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<(), Self::Error> {
        self.serialize_str(variant)
    }

    #[inline]
    fn serialize_newtype_struct<U: ?Sized + ser::Serialize>(
        self,
        name: &'static str,
        value: &U,
    ) -> Result<(), Self::Error> {
        if name == "@@SIMPLETYPE@@" {
            use serde::ser::Error as _;

            let v = crate::Value::serialized(value).map_err(Error::custom)?;
            let v = v
                .as_integer()
                .ok_or_else(|| Error::custom("Internal error handling simple types"))?;
            let v = u8::try_from(v).map_err(Error::custom)?;
            Ok(self.0.push(Header::Simple(v))?)
        } else {
            value.serialize(self)
        }
    }

    #[inline]
    fn serialize_newtype_variant<U: ?Sized + ser::Serialize>(
        self,
        name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &U,
    ) -> Result<(), Self::Error> {
        if name == "@@ST@@" && variant == "@@SIMPLETYPE@@" {
            use serde::ser::Error as _;

            let v = crate::Value::serialized(value).map_err(Error::custom)?;
            let v = v
                .as_integer()
                .ok_or_else(|| Error::custom("Internal error handling simple types"))?;
            let v = u8::try_from(v).map_err(Error::custom)?;
            return Ok(self.0.push(Header::Simple(v))?);
        } else if name != "@@TAG@@" || variant != "@@UNTAGGED@@" {
            self.0.push(Header::Map(Some(1)))?;
            self.serialize_str(variant)?;
        }

        value.serialize(self)
    }

    #[inline]
    fn serialize_seq(self, length: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        self.0.push(Header::Array(length))?;
        Ok(ZeroCopyCollectionSerializer {
            encoder: self,
            ending: length.is_none(),
            tag: false,
        })
    }

    #[inline]
    fn serialize_tuple(self, length: usize) -> Result<Self::SerializeTuple, Self::Error> {
        self.serialize_seq(Some(length))
    }

    #[inline]
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        length: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        self.serialize_seq(Some(length))
    }

    #[inline]
    fn serialize_tuple_variant(
        self,
        name: &'static str,
        _index: u32,
        variant: &'static str,
        length: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        match (name, variant) {
            ("@@TAG@@", "@@TAGGED@@") => Ok(ZeroCopyCollectionSerializer {
                encoder: self,
                ending: false,
                tag: true,
            }),

            _ => {
                self.0.push(Header::Map(Some(1)))?;
                self.serialize_str(variant)?;
                self.0.push(Header::Array(Some(length)))?;
                Ok(ZeroCopyCollectionSerializer {
                    encoder: self,
                    ending: false,
                    tag: false,
                })
            }
        }
    }

    #[inline]
    fn serialize_map(self, length: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        self.0.push(Header::Map(length))?;
        Ok(ZeroCopyCollectionSerializer {
            encoder: self,
            ending: length.is_none(),
            tag: false,
        })
    }

    #[inline]
    fn serialize_struct(
        self,
        _name: &'static str,
        length: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        self.0.push(Header::Map(Some(length)))?;
        Ok(ZeroCopyCollectionSerializer {
            encoder: self,
            ending: false,
            tag: false,
        })
    }

    #[inline]
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        length: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        self.0.push(Header::Map(Some(1)))?;
        self.serialize_str(variant)?;
        self.0.push(Header::Map(Some(length)))?;
        Ok(ZeroCopyCollectionSerializer {
            encoder: self,
            ending: false,
            tag: false,
        })
    }

    #[inline]
    fn is_human_readable(&self) -> bool {
        false
    }
}

macro_rules! end {
    () => {
        #[inline]
        fn end(self) -> Result<(), Self::Error> {
            if self.ending {
                self.encoder.0.push(Header::Break)?;
            }
            Ok(())
        }
    };
}

#[doc(hidden)]
pub struct ZeroCopyCollectionSerializer<'a, 'b, W: Write + WriteByteSlice<'a>> {
    encoder: &'b mut ByteSliceSerializer<'a, W>,
    ending: bool,
    tag: bool,
}

impl<'a, 'b, W: Write + WriteByteSlice<'a>> ser::SerializeSeq
    for ZeroCopyCollectionSerializer<'a, 'b, W>
where
    W::Error: core::fmt::Debug,
{
    type Ok = ();
    type Error = Error<W::Error>;

    #[inline]
    fn serialize_element<U: ?Sized + ser::Serialize>(
        &mut self,
        value: &U,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut *self.encoder)
    }

    end!();
}

impl<'a, 'b, W: Write + WriteByteSlice<'a>> ser::SerializeTuple
    for ZeroCopyCollectionSerializer<'a, 'b, W>
where
    W::Error: core::fmt::Debug,
{
    type Ok = ();
    type Error = Error<W::Error>;

    #[inline]
    fn serialize_element<U: ?Sized + ser::Serialize>(
        &mut self,
        value: &U,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut *self.encoder)
    }

    end!();
}

impl<'a, 'b, W: Write + WriteByteSlice<'a>> ser::SerializeTupleStruct
    for ZeroCopyCollectionSerializer<'a, 'b, W>
where
    W::Error: core::fmt::Debug,
{
    type Ok = ();
    type Error = Error<W::Error>;

    #[inline]
    fn serialize_field<U: ?Sized + ser::Serialize>(
        &mut self,
        value: &U,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut *self.encoder)
    }

    end!();
}

impl<'a, 'b, W: Write + WriteByteSlice<'a>> ser::SerializeTupleVariant
    for ZeroCopyCollectionSerializer<'a, 'b, W>
where
    W::Error: core::fmt::Debug,
{
    type Ok = ();
    type Error = Error<W::Error>;

    #[inline]
    fn serialize_field<U: ?Sized + ser::Serialize>(
        &mut self,
        value: &U,
    ) -> Result<(), Self::Error> {
        if !self.tag {
            return value.serialize(&mut *self.encoder);
        }

        self.tag = false;
        match value.serialize(crate::tag::Serializer) {
            Ok(x) => Ok(self.encoder.0.push(Header::Tag(x))?),
            _ => Err(Error::Value("expected tag".into())),
        }
    }

    end!();
}

impl<'a, 'b, W: Write + WriteByteSlice<'a>> ser::SerializeMap
    for ZeroCopyCollectionSerializer<'a, 'b, W>
where
    W::Error: core::fmt::Debug,
{
    type Ok = ();
    type Error = Error<W::Error>;

    #[inline]
    fn serialize_key<U: ?Sized + ser::Serialize>(&mut self, key: &U) -> Result<(), Self::Error> {
        key.serialize(&mut *self.encoder)
    }

    #[inline]
    fn serialize_value<U: ?Sized + ser::Serialize>(
        &mut self,
        value: &U,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut *self.encoder)
    }

    end!();
}

impl<'a, 'b, W: Write + WriteByteSlice<'a>> ser::SerializeStruct
    for ZeroCopyCollectionSerializer<'a, 'b, W>
where
    W::Error: core::fmt::Debug,
{
    type Ok = ();
    type Error = Error<W::Error>;

    #[inline]
    fn serialize_field<U: ?Sized + ser::Serialize>(
        &mut self,
        key: &'static str,
        value: &U,
    ) -> Result<(), Self::Error> {
        key.serialize(&mut *self.encoder)?;
        value.serialize(&mut *self.encoder)?;
        Ok(())
    }

    end!();
}

impl<'a, 'b, W: Write + WriteByteSlice<'a>> ser::SerializeStructVariant
    for ZeroCopyCollectionSerializer<'a, 'b, W>
where
    W::Error: core::fmt::Debug,
{
    type Ok = ();
    type Error = Error<W::Error>;

    #[inline]
    fn serialize_field<U: ?Sized + ser::Serialize>(
        &mut self,
        key: &'static str,
        value: &U,
    ) -> Result<(), Self::Error> {
        key.serialize(&mut *self.encoder)?;
        value.serialize(&mut *self.encoder)
    }

    end!();
}

/// Serializes `value` as CBOR into a scatter-gather list of byte slices.
///
/// The `buf` scratch buffer stores the encoded form of scalar values (integers,
/// floats, booleans, CBOR headers). It must be large enough to hold all such
/// encoded bytes; returns an [`EndOfFile`] error otherwise.
///
/// String and byte-slice values are referenced in-place (zero-copy): the returned
/// `Vec` contains direct pointers into the original data rather than copies.
///
/// The returned `Vec<&'a [u8]>` is a scatter-gather list over the CBOR encoding.
/// Concatenating all slices in order yields a valid CBOR byte stream.
///
/// # Safety
///
/// Relies on the safety invariant of [`ByteSliceSerializer`]: every `Serialize`
/// implementation of types reachable from `value` must only pass slices borrowed
/// from the value itself to `serialize_str` / `serialize_bytes`.
pub fn into_byte_slice<'a, T: ?Sized + ser::Serialize>(
    value: &'a T,
    buf: &'a mut [u8],
) -> Result<Vec<&'a [u8]>, Error<EndOfFile>> {
    let writer = ByteSliceWriter::new(buf, Vec::new());
    let mut ser = ByteSliceSerializer::from(writer);
    value.serialize(&mut ser)?;
    Ok(ser.into_inner().into_vec())
}
