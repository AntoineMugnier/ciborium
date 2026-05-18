// SPDX-License-Identifier: Apache-2.0

//! Serde serialization support for CBOR

mod error;
mod general_purpose;
mod zero_copy;

pub use error::Error;
#[cfg(feature = "std")]
pub use general_purpose::into_vec;
pub use general_purpose::into_writer;
#[doc(hidden)]
pub use zero_copy::ZeroCopyCollectionSerializer;
pub use zero_copy::{into_byte_slice, ByteSliceSerializer};
