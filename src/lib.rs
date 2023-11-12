#![doc = include_str!("../README.md")]

use bitvec::prelude::Msb0;
use bitvec::vec::BitVec;
use std::collections::HashMap;
use std::io::Read;
use std::io;

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// This stores *bits*, not bytes.
    Blob(BitVec<u8, Msb0>),
    Text(String),
    Array(Vec<Value>),
    Record(HashMap<String, Value>),
    Integer(i128),
    #[allow(deprecated)]
    DecimalFloat(DecimalFloat),
    Bool(bool),
}

/// Conversion from `f32`/`f64` is going to be tricky, see the [`ryu`](https://lib.rs/crates/ryu) crate.
/// ```js
/// value = coefficient * power(10, exponent)
/// ```
#[derive(Debug, Clone, PartialEq)]
#[deprecated(note = "this unimplemented, and likely to be removed")]
pub struct DecimalFloat {
    pub exponent: i32,
    pub coefficient: i64,
}

fn serialize_signed_preamble(header: u8, value: i128, into: &mut Vec<u8>) {
    let (sign_bit, value) = if value < 0 {
        (1, -value as u128)
    } else {
        (0, value as u128)
    };

    let minimum_bit_len = 128 - value.leading_zeros();
    let mut bit_len = 3 + ((minimum_bit_len.saturating_sub(3) + 6) / 7) * 7;

    let next = (value >> (bit_len as i32 - 3) & 0b111) as u8;
    into.push(header | (sign_bit << 3) | next | if bit_len > 3 { 0b0001_0000 } else { 0 });
    bit_len -= 3;
    serialize_integer_continuation(value, bit_len, into);
}

fn serialize_unsigned_preamble(header: u8, value: u128, into: &mut Vec<u8>) {
    let minimum_bit_len = 128 - value.leading_zeros();
    let mut bit_len = 4 + ((minimum_bit_len.saturating_sub(4) + 6) / 7) * 7;

    let next = (value >> (bit_len as i32 - 4) & 0b1111) as u8;
    into.push(header | next | if bit_len > 4 { 0b0001_0000 } else { 0 });
    bit_len -= 4;
    serialize_integer_continuation(value, bit_len, into);
}

fn serialize_integer_continuation(value: u128, mut bit_len: u32, into: &mut Vec<u8>) {
    while bit_len > 0 {
        let next = (value >> (bit_len as i32 - 7)) as u8 & 0b111_1111;
        let c = if bit_len > 7 { 0b1000_0000 } else { 0 };
        into.push(next | c);
        if bit_len <= 7 { break; }
        bit_len -= 7;
    }
}

impl Value {
    pub fn serialize_into(&self, into: &mut Vec<u8>) {
        match self {
            Value::Blob(val) => {
                serialize_unsigned_preamble(0, val.len() as u128, into);
                debug_assert_eq!((val.len() + 7) / 8, val.as_raw_slice().len());
                into.extend_from_slice(val.as_raw_slice());
            },
            Value::Text(val) => {
                serialize_string(val, into);
            },
            Value::Array(val) => {
                serialize_unsigned_preamble(0b0100_0000, val.len() as u128, into);
                for v in val {
                    v.serialize_into(into);
                }
            },
            Value::Record(val) => {
                serialize_unsigned_preamble(0b0110_0000, val.len() as u128, into);
                for (k, v) in val {
                    serialize_string(k, into);
                    v.serialize_into(into);
                }
            },
            Value::Integer(val) => {
                serialize_signed_preamble(0b1000_0000, *val, into);
            },
            Value::DecimalFloat(_val) => {
                unimplemented!("this platform uses IEEE754 floats, not DEC64 floats");
            },
            Value::Bool(val) => {
                into.push(0b1100_0000 | u8::from(*val));
            },
        }
    }

    pub fn parse_from<R: Read>(reader: &mut R) -> Result<Self, io::Error> {
        let mut preamble = 0;
        reader.read_exact(std::slice::from_mut(&mut preamble))?;
        let kind = preamble & 0b1110_0000;
        Ok(match kind {
            0b0000_0000 => {
                let len = parse_len(preamble, reader)?;
                let len_bytes = (len + 7) / 8;
                let mut out = Vec::new();
                reader.take(len_bytes as u64).read_to_end(&mut out)?;
                if out.len() != len_bytes {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                let mut out = BitVec::from_vec(out);
                if len & 7 != 0 {
                    out.truncate(len);
                }
                Self::Blob(out)
            },
            0b0010_0000 => {
                let len = parse_len(preamble, reader)?;
                let mut out = String::with_capacity(len.min(1 << 20));
                for _ in 0..len {
                    out.push(read_kim_char(reader)?);
                }
                Self::Text(out)
            },
            0b0100_0000 => {
                let len = parse_len(preamble, reader)?;
                let mut out = Vec::with_capacity(len.min(1 << 18));
                for _ in 0..len {
                    out.push(Self::parse_from(reader)?);
                }
                Self::Array(out)
            },
            0b0110_0000 => {
                let len = parse_len(preamble, reader)?;
                let mut out = HashMap::with_capacity(len.min(1 << 16));
                for _ in 0..len {
                    let k = Self::parse_from(reader)?;
                    let v = Self::parse_from(reader)?;
                    if let Value::Text(k) = k {
                        out.insert(k, v);
                    } else {
                        return Err(io::Error::from(io::ErrorKind::InvalidData).into());
                    }
                }
                Self::Record(out)
            },
            0b1000_0000 => {
                let sign = preamble & 0b000_1000;
                let mut val = (preamble & 0b000_0111) as u128;
                if preamble & 0b0001_0000 != 0 {
                    loop {
                        val <<= 7;
                        let mut next = 0;
                        reader.read_exact(std::slice::from_mut(&mut next))?;
                        val |= (next & 0b0111_1111) as u128;
                        if next & 0b1000_0000 == 0 {
                            break;
                        }
                    }
                }
                Self::Integer(if sign == 0 { val as i128 } else { -(val as i128) })
            },
            0b1010_0000 => unimplemented!("this platform uses IEEE754 floats, not DEC64 floats"),
            0b1100_0000 => {
                let val = preamble & 0b0001_1111;
                match val {
                    0 => Self::Bool(false),
                    1 => Self::Bool(true),
                    _ => return Err(io::ErrorKind::Unsupported.into()),
                }
            },
            _ => return Err(io::ErrorKind::InvalidData.into()),
        })
    }
}

#[inline(never)]
fn serialize_string(val: &str, into: &mut Vec<u8>) {
    let char_len = val.chars().count();
    serialize_unsigned_preamble(0b0010_0000, char_len as u128, into);
    for c in val.chars() {
        write_kim_char(c, into);
    }
}

fn read_kim_char<R: Read>(reader: &mut R) -> Result<char, io::Error> {
    let mut val = 0;
    loop {
        let mut next = 0;
        reader.read_exact(std::slice::from_mut(&mut next))?;
        val |= next as u32 & 0b0111_1111;
        if next & 0b1000_0000 == 0 {
            return char::from_u32(val).ok_or(io::Error::from(io::ErrorKind::InvalidData));
        }
        val <<= 7;
    }
}

fn write_kim_char(code_point: char, into: &mut Vec<u8>) {
    let val = code_point as u32;
    if val < 0x80 {
        into.push(val as u8);
    } else {
        if val >= 1 << 14 {
            into.push(0b1000_0000 | (val >> 14) as u8);
        }
        into.push(0b1000_0000 | (val >> 7) as u8);
        into.push(val as u8 & 0b0111_1111);
    }
}

#[inline(never)]
fn parse_len<R: Read>(preamble: u8, reader: &mut R) -> Result<usize, io::Error> {
    let mut len = preamble as usize & 0b000_1111;
    if preamble & 0b0001_0000 != 0 {
        loop {
            len <<= 7;
            let mut next = 0;
            reader.read_exact(std::slice::from_mut(&mut next))?;
            len |= (next & 0b0111_1111) as usize;
            if next & 0b1000_0000 == 0 {
                break;
            }
        }
    }
    Ok(len)
}

#[cfg(test)]
#[track_caller]
fn assert_serializes(val: Value, nota: &[u8]) {
    let mut tmp = nota;
    let mut out = Vec::new();
    val.serialize_into(&mut out);
    if out != nota {
        panic!("Expected {}\n     got {}\n{}",
            nota.iter().map(|n| format!("{n:8b}, ")).collect::<String>(),
            out.iter().map(|n| format!("{n:8b}, ")).collect::<String>(),
            out.iter().map(|n| format!("0x{n:02x}, ")).collect::<String>(),
        );
    }
    assert_eq!(Value::parse_from(&mut tmp).unwrap(), val);
}

#[test]
fn integer() {
    assert_serializes(Value::Integer(2023), &[0x90, 0x8F, 0x67]);
    assert_serializes(Value::Integer(0), &[0x80]);
    assert_serializes(Value::Integer(-1), &[0x89]);
    assert_serializes(Value::Integer(0b1), &[0x81]);
    assert_serializes(Value::Integer(0b101110), &[0x90, 0x2e]);
    assert_serializes(Value::Integer(0b1011101111101), &[0x90, 0xae, 0x7d]);
    assert_serializes(Value::Integer(0b101110111110111111), &[0x90, 0x8b, 0xdf, 0x3f]);
    assert_serializes(Value::Integer(0b101110111110111111111), &[0x90, 0xdd, 0xfb, 0x7f]);
    assert_serializes(Value::Integer(0b1001110111110111111111), &[0x91, 0x9d, 0xfb, 0x7f]);
    assert_serializes(Value::Integer(i128::MAX), &[0x91, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]);
}

#[test]
fn bool() {
    assert_serializes(Value::Bool(false), &[0xC0]);
    assert_serializes(Value::Bool(true), &[0xC1]);
}

#[test]
fn text() {
    assert_serializes(Value::Text("cat".into()), &[0x23, 0x63, 0x61, 0x74]);
    assert_serializes(Value::Text("".into()), &[0x20]);
    assert_serializes(Value::Text("☃★♲".into()), &[0x23, 0xCC, 0x03, 0xCC, 0x05, 0xCC, 0x72]);
    assert_serializes(Value::Text("𓂀𓃠𓅣𓂻𓂺𓁟𓂑𓃻𓇼𓊽𓂭𓎆𓍢𓏢𓐠".into()), &[0x2F, 0x84, 0xE1, 0x00, 0x84, 0xE1, 0x60, 0x84, 0xE2, 0x63, 0x84, 0xE1, 0x3B, 0x84, 0xE1, 0x3A, 0x84, 0xE0, 0x5F, 0x84, 0xE1, 0x11, 0x84, 0xE1, 0x7B, 0x84, 0xE3, 0x7C, 0x84, 0xE5, 0x3D, 0x84, 0xE1, 0x2D, 0x84, 0xE7, 0x06, 0x84, 0xE6, 0x62, 0x84, 0xE7, 0x62, 0x84, 0xE8, 0x20]);
}

#[test]
fn array() {
    assert_serializes(Value::Array(vec![Value::Bool(false), Value::Integer(2023)]), &[0b1000010, 0xC0, 0x90, 0x8F, 0x67]);
}

// likely incorrect, because the spec has no examples to test against
#[test]
fn blob() {
    let mut bitblob = BitVec::new();
    bitblob.extend([0x55_u8]);
    bitblob.push(true);
    bitblob.push(true);
    bitblob.push(false);

    assert_serializes(Value::Blob(bitblob), &[0b1011, 0b1010101, 0b11000000]);
    assert_serializes(Value::Blob(vec![1u8,2,3].try_into().unwrap()), &[0b10000, 0b11000, 1,2,3]);
}

// may be incorrect, because the spec has no examples to test against
#[test]
fn record() {
    let mut hash = HashMap::new();
    hash.insert("Hello".into(), Value::Integer(123456789));

    assert_serializes(Value::Record(hash), &[0x61, 0x25, 0x48, 0x65, 0x6c, 0x6c, 0x6f, 0x90, 0xba, 0xef, 0x9a, 0x15]);
}
