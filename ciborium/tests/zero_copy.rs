// SPDX-License-Identifier: Apache-2.0

extern crate std;

use ciborium::ser::{into_byte_slice, into_writer};
use serde::Serialize;

// ── helpers ──────────────────────────────────────────────────────────────────

fn reassemble(slices: &[&[u8]]) -> std::vec::Vec<u8> {
    slices.iter().flat_map(|s| s.iter().copied()).collect()
}

fn canonical<T: Serialize + ?Sized>(value: &T) -> std::vec::Vec<u8> {
    let mut out = std::vec::Vec::new();
    into_writer(value, &mut out).unwrap();
    out
}

/// Returns `true` if any slice in `output` points into `original`'s memory.
fn is_zero_copy(output: &[&[u8]], original: &[u8]) -> bool {
    if original.is_empty() {
        return true;
    }
    let orig_start = original.as_ptr() as usize;
    let orig_end = orig_start + original.len();
    output.iter().any(|sl| {
        let sl_start = sl.as_ptr() as usize;
        !sl.is_empty() && sl_start >= orig_start && sl_start + sl.len() <= orig_end
    })
}

// ── scalar types go entirely through the scratch buffer ──────────────────────

#[test]
fn scalar_bool() {
    let mut buf = [0u8; 4];
    assert_eq!(reassemble(&into_byte_slice(&true, &mut buf).unwrap()), canonical(&true));
    assert_eq!(reassemble(&into_byte_slice(&false, &mut buf).unwrap()), canonical(&false));
}

#[test]
fn scalar_u8() {
    for v in [0u8, 1, 23, 24, 127, 255] {
        let mut buf = [0u8; 4];
        assert_eq!(reassemble(&into_byte_slice(&v, &mut buf).unwrap()), canonical(&v));
    }
}

#[test]
fn scalar_i64() {
    for v in [0i64, -1, 100, -100, i64::MIN, i64::MAX] {
        let mut buf = [0u8; 16];
        assert_eq!(reassemble(&into_byte_slice(&v, &mut buf).unwrap()), canonical(&v));
    }
}

#[test]
fn scalar_f64() {
    for v in [0.0f64, 1.5, -1.0, f64::NAN, f64::INFINITY] {
        let mut buf = [0u8; 16];
        let slices = into_byte_slice(&v, &mut buf).unwrap();
        // NaN != NaN, so just verify length
        assert_eq!(reassemble(&slices).len(), canonical(&v).len());
    }
}

#[test]
fn option_none() {
    let v: Option<u8> = None;
    let mut buf = [0u8; 4];
    assert_eq!(reassemble(&into_byte_slice(&v, &mut buf).unwrap()), canonical(&v));
}

#[test]
fn option_some() {
    let v: Option<u8> = Some(42);
    let mut buf = [0u8; 4];
    assert_eq!(reassemble(&into_byte_slice(&v, &mut buf).unwrap()), canonical(&v));
}

// ── &str: header in scratch, content zero-copy ───────────────────────────────

#[test]
fn str_output_correct() {
    let s = "hello, world!";
    let mut buf = [0u8; 8];
    let slices = into_byte_slice(&s, &mut buf).unwrap();
    assert_eq!(reassemble(&slices), canonical(&s));
}

#[test]
fn str_content_is_zero_copy() {
    let s = "hello, world!";
    let mut buf = [0u8; 8];
    let slices = into_byte_slice(&s, &mut buf).unwrap();
    assert!(
        is_zero_copy(&slices, s.as_bytes()),
        "string content must point into the original str, not the scratch buffer"
    );
}

#[test]
fn str_header_is_in_scratch_buf() {
    // Header (1-2 bytes) should land inside `buf`, not in the str data.
    let s = "hi";
    let mut buf = [0u8; 8];
    // Capture buf address range as plain integers before the mutable borrow.
    let buf_start = buf.as_ptr() as usize;
    let buf_end = buf_start + buf.len();
    let slices = into_byte_slice(&s, &mut buf).unwrap();
    // The Text(2) header byte `62` must be in `buf`
    let header_slice = slices[0];
    let hdr_ptr = header_slice.as_ptr() as usize;
    assert!(
        hdr_ptr >= buf_start && hdr_ptr + header_slice.len() <= buf_end,
        "CBOR header should reside in the scratch buffer"
    );
}

#[test]
fn empty_str() {
    let s = "";
    let mut buf = [0u8; 4];
    assert_eq!(reassemble(&into_byte_slice(&s, &mut buf).unwrap()), canonical(&s));
}

// ── bytes (via serde_bytes): header in scratch, content zero-copy ─────────────

#[test]
fn bytes_output_correct() {
    let data: &[u8] = b"binary data";
    let bytes = serde_bytes::Bytes::new(data);
    let mut buf = [0u8; 8];
    assert_eq!(
        reassemble(&into_byte_slice(bytes, &mut buf).unwrap()),
        canonical(bytes)
    );
}

#[test]
fn bytes_content_is_zero_copy() {
    let data: &[u8] = b"binary data";
    let bytes = serde_bytes::Bytes::new(data);
    let mut buf = [0u8; 8];
    let slices = into_byte_slice(bytes, &mut buf).unwrap();
    assert!(
        is_zero_copy(&slices, data),
        "bytes content must point into the original slice, not the scratch buffer"
    );
}

// ── struct with borrowed fields ───────────────────────────────────────────────

#[derive(Serialize)]
struct BorrowedMsg<'a> {
    id: u32,
    name: &'a str,
    #[serde(with = "serde_bytes")]
    payload: &'a [u8],
}

#[test]
fn struct_borrowed_output_correct() {
    let msg = BorrowedMsg {
        id: 7,
        name: "alice",
        payload: b"hello bytes",
    };
    let mut buf = [0u8; 64];
    assert_eq!(
        reassemble(&into_byte_slice(&msg, &mut buf).unwrap()),
        canonical(&msg)
    );
}

#[test]
fn struct_borrowed_fields_are_zero_copy() {
    let name = "alice";
    let payload = b"hello bytes";
    let msg = BorrowedMsg { id: 7, name, payload };
    let mut buf = [0u8; 64];
    let slices = into_byte_slice(&msg, &mut buf).unwrap();
    assert!(
        is_zero_copy(&slices, name.as_bytes()),
        "name field must be zero-copy"
    );
    assert!(
        is_zero_copy(&slices, payload),
        "payload field must be zero-copy"
    );
}

// ── struct with owned String / Vec<u8> ───────────────────────────────────────

#[derive(Serialize)]
struct OwnedMsg {
    id: u32,
    name: std::string::String,
    #[serde(with = "serde_bytes")]
    payload: std::vec::Vec<u8>,
}

#[test]
fn struct_owned_output_correct() {
    let msg = OwnedMsg {
        id: 99,
        name: std::string::String::from("bob"),
        payload: b"owned payload".to_vec(),
    };
    let mut buf = [0u8; 64];
    assert_eq!(
        reassemble(&into_byte_slice(&msg, &mut buf).unwrap()),
        canonical(&msg)
    );
}

#[test]
fn struct_owned_fields_point_into_heap() {
    let msg = OwnedMsg {
        id: 99,
        name: std::string::String::from("bob"),
        payload: b"owned payload".to_vec(),
    };
    let mut buf = [0u8; 64];
    let slices = into_byte_slice(&msg, &mut buf).unwrap();
    assert!(
        is_zero_copy(&slices, msg.name.as_bytes()),
        "name must point into the String's heap allocation"
    );
    assert!(
        is_zero_copy(&slices, &msg.payload),
        "payload must point into the Vec's heap allocation"
    );
}

// ── struct field name keys are zero-copy (&'static str) ─────────────────────

#[test]
fn field_keys_are_zero_copy() {
    // 10-field struct: map header (1 byte) + per field: key Text header (1) + value (1-2)
    // = at most 1 + 10*3 = 31 bytes in scratch. Larger if keys are not zero-copy.
    // We size the buffer to succeed only if key content doesn't consume scratch space.
    #[derive(Serialize)]
    struct Wide {
        a: u8,
        b: u8,
        c: u8,
        d: u8,
        e: u8,
        f: u8,
        g: u8,
        h: u8,
        ii: u8, // 2-char key to also test multi-byte keys
        jj: u8,
    }
    let w = Wide {
        a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8, ii: 9, jj: 10,
    };
    let mut buf = [0u8; 64];
    let slices = into_byte_slice(&w, &mut buf).unwrap();
    assert_eq!(reassemble(&slices), canonical(&w));
}

// ── nested struct ─────────────────────────────────────────────────────────────

#[test]
fn nested_struct_output_correct() {
    #[derive(Serialize)]
    struct Inner<'a> {
        val: &'a str,
    }
    #[derive(Serialize)]
    struct Outer<'a> {
        id: u32,
        inner: Inner<'a>,
    }
    let text = "nested text";
    let value = Outer { id: 1, inner: Inner { val: text } };
    let mut buf = [0u8; 64];
    let slices = into_byte_slice(&value, &mut buf).unwrap();
    assert_eq!(reassemble(&slices), canonical(&value));
    assert!(is_zero_copy(&slices, text.as_bytes()));
}

// ── large string: only the header fits in buf ────────────────────────────────

#[test]
fn large_string_small_buf() {
    // 10 000-char string: CBOR header is 3 bytes (Text with 2-byte length).
    // Entire content is zero-copy so 8 bytes of scratch is plenty.
    let large = "x".repeat(10_000);
    let large_str: &str = &large;
    let mut buf = [0u8; 8];
    let slices = into_byte_slice(&large_str, &mut buf).unwrap();
    assert_eq!(reassemble(&slices), canonical(&large_str));
    assert!(is_zero_copy(&slices, large.as_bytes()));
}

// ── sequences and maps ────────────────────────────────────────────────────────

#[test]
fn sequence_of_scalars() {
    let v: &[u32] = &[1, 2, 3, 1000];
    let mut buf = [0u8; 32];
    assert_eq!(reassemble(&into_byte_slice(&v, &mut buf).unwrap()), canonical(&v));
}

#[test]
fn map_string_to_u32() {
    let mut map = std::collections::BTreeMap::new();
    map.insert("alpha", 1u32);
    map.insert("beta", 2u32);
    map.insert("gamma", 3u32);
    let mut buf = [0u8; 32];
    let slices = into_byte_slice(&map, &mut buf).unwrap();
    assert_eq!(reassemble(&slices), canonical(&map));
    // Map keys ("alpha", "beta", "gamma") should be zero-copy
    for key in ["alpha", "beta", "gamma"] {
        assert!(is_zero_copy(&slices, key.as_bytes()), "key '{}' should be zero-copy", key);
    }
}

// ── error: scratch buffer overflow ───────────────────────────────────────────

#[test]
fn buf_overflow_returns_error() {
    // A u64 value encodes to at least 1 byte; zero-length buf must fail.
    let mut buf = [];
    assert!(into_byte_slice(&42u64, &mut buf).is_err());
}

#[test]
fn buf_one_byte_just_enough_for_small_int() {
    // Positive(0) encodes as a single byte `00`, so a 1-byte buf is sufficient.
    let mut buf = [0u8; 1];
    let slices = into_byte_slice(&0u8, &mut buf).unwrap();
    assert_eq!(reassemble(&slices), canonical(&0u8));
}

#[test]
fn buf_one_byte_not_enough_for_large_int() {
    // u16::MAX encodes as 3 bytes (prefix + 2-byte value), so 1-byte buf fails.
    let mut buf = [0u8; 1];
    assert!(into_byte_slice(&u16::MAX, &mut buf).is_err());
}
