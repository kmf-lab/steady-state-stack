//////////////////////////////////////////////////////////
// FIX FAST (FIX Adapted for Streaming) Decoder and Encoder
//////////////////////////////////////////////////////////////////////////
// Write signed long using variable length encoding as defined in FAST spec
//  NOTE: do not modify this but do duplicate this to build an i128 version.
//////////////////////////////////////////////////////////////////////////

use bytes::{Buf, BufMut, Bytes};
use bytes::BytesMut;

/// Reads a signed long from the provided byte buffer using variable length encoding as defined in the FAST spec.
///
/// # Arguments
///
/// * `byte_buffer` - A mutable reference to a `Bytes` buffer from which to read the signed long.
///
/// # Returns
///
/// * `Option<i64>` - Returns `Some(i64)` if the read operation is successful, or `None` if the buffer is empty or data is invalid.
pub fn read_long_signed(byte_buffer: &mut Bytes) -> Option<i64> {
    let initial_remaining: usize = byte_buffer.remaining();
    if initial_remaining > 0 {
        let v = byte_buffer.get_i8();
        let accumulator = ((!(((v >> 6) & 1) - 1)) as i64) & (0xFFFFFFFFFFFFFF80u64 as i64);
        if v < 0 {
            Some(accumulator | (v as i64 & 0x7F))
        } else {
            read_long_signed_tail((accumulator | v as i64) << 7, byte_buffer, initial_remaining)
        }
    } else {
        None
    }
}

/// Reads the tail part of a signed long using variable length encoding.
///
/// # Arguments
///
/// * `a` - The accumulated value so far.
/// * `byte_buffer` - A mutable reference to a `Bytes` buffer from which to read the signed long.
/// * `initial_remaining` - The initial number of remaining bytes in the buffer.
///
/// # Returns
///
/// * `Option<i64>` - Returns `Some(i64)` if the read operation is successful, or `None` if data is invalid.
fn read_long_signed_tail(a: i64, byte_buffer: &mut Bytes, initial_remaining: usize) -> Option<i64> {
    let remaining: usize = byte_buffer.remaining();
    if remaining > 0 {
        let v = byte_buffer.get_i8();
        if v < 0 {
            Some(a | (v as i64 & 0x7F))
        } else if initial_remaining - remaining > 10 {
            None // Found bad data, stop reading
        } else {
            read_long_signed_tail((a | v as i64) << 7, byte_buffer, initial_remaining)
        }
    } else {
        None
    }
}

/// Reads an unsigned long from the provided byte buffer using variable length encoding.
///
/// # Arguments
///
/// * `byte_buffer` - A mutable reference to a `Bytes` buffer from which to read the unsigned long.
///
/// # Returns
///
/// * `Option<u64>` - Returns `Some(u64)` if the read operation is successful, or `None` if the buffer is empty or data is invalid.
pub fn read_long_unsigned(byte_buffer: &mut Bytes) -> Option<u64> {
    let mut value: u64 = 0;
    let mut byte_count = 0;

    while byte_buffer.has_remaining() {
        let byte = byte_buffer.get_u8();
        value = (value << 7) | ((byte & 0x7F) as u64);

        // Check if the stop bit is set
        if byte & 0x80 != 0 {
            // If high bit is set, stop reading further
            return Some(value);
        }

        // Prevent reading more than expected for a u64 value
        byte_count += 1;
        if byte_count > 10 {
            return None; // Too many bytes, likely incorrect data
        }
    }

    // If the buffer ends before the sequence is complete, return None
    None
}

/// Writes a positive signed long value to the provided byte buffer using variable length encoding.
///
/// # Arguments
///
/// * `value` - The value to be written.
/// * `byte_buffer` - A mutable reference to a `BytesMut` buffer to which the value will be written.
fn write_long_signed_pos(value: u64, byte_buffer: &mut BytesMut) {
    if value >= 0x0000000000000040 {
        if value >= 0x0000000000002000 {
            if value >= 0x0000000000100000 {
                if value >= 0x0000000008000000 {
                    if value >= 0x0000000400000000 {
                        if value >= 0x0000020000000000 {
                            if value >= 0x0001000000000000 {
                                if value >= 0x0080000000000000 {
                                    if value >= 0x4000000000000000 {
                                        byte_buffer.put_u8(((value >> 63) & 0x7F) as u8);
                                    }
                                    byte_buffer.put_u8(((value >> 56) & 0x7F) as u8);
                                }
                                byte_buffer.put_u8(((value >> 49) & 0x7F) as u8);
                            }
                            byte_buffer.put_u8(((value >> 42) & 0x7F) as u8);
                        }
                        byte_buffer.put_u8(((value >> 35) & 0x7F) as u8);
                    }
                    byte_buffer.put_u8(((value >> 28) & 0x7F) as u8);
                }
                byte_buffer.put_u8(((value >> 21) & 0x7F) as u8);
            }
            byte_buffer.put_u8(((value >> 14) & 0x7F) as u8);
        }
        byte_buffer.put_u8(((value >> 7) & 0x7F) as u8);
    }
    // Always write the last byte
    byte_buffer.put_u8(((value & 0x7F) | 0x80) as u8);
}

/// Writes a negative signed long value to the provided byte buffer using variable length encoding.
///
/// # Arguments
///
/// * `value` - The value to be written.
/// * `byte_buffer` - A mutable reference to a `BytesMut` buffer to which the value will be written.
fn write_long_signed_neg(value: i64, byte_buffer: &mut BytesMut) {
    let absv = (-value) as u64;
    if absv > 0x0000000000000040 {
        if absv > 0x0000000000002000 {
            if absv > 0x0000000000100000 {
                if absv > 0x0000000008000000 {
                    if absv > 0x0000000400000000 {
                        if absv > 0x0000020000000000 {
                            if absv > 0x0001000000000000 {
                                if absv > 0x0080000000000000 {
                                    if absv > 0x4000000000000000 {
                                        byte_buffer.put_u8(((value >> 63) & 0x7F) as u8);
                                    }
                                    byte_buffer.put_u8(((value >> 56) & 0x7F) as u8);
                                }
                                byte_buffer.put_u8(((value >> 49) & 0x7F) as u8);
                            }
                            byte_buffer.put_u8(((value >> 42) & 0x7F) as u8);
                        }
                        byte_buffer.put_u8(((value >> 35) & 0x7F) as u8);
                    }
                    byte_buffer.put_u8(((value >> 28) & 0x7F) as u8);
                }
                byte_buffer.put_u8(((value >> 21) & 0x7F) as u8);
            }
            byte_buffer.put_u8(((value >> 14) & 0x7F) as u8);
        }
        byte_buffer.put_u8(((value >> 7) & 0x7F) as u8);
    }
    byte_buffer.put_u8(((value & 0x7F) | 0x80) as u8);
}

/// Writes an unsigned long value to the provided byte buffer using variable length encoding.
///
/// # Arguments
///
/// * `value` - The value to be written.
/// * `byte_buffer` - A mutable reference to a `BytesMut` buffer to which the value will be written.
pub fn write_long_unsigned(value: u64, byte_buffer: &mut BytesMut) {
    write_long_signed_pos(value, byte_buffer);
}

/// Writes a signed long value to the provided byte buffer using variable length encoding.
///
/// # Arguments
///
/// * `value` - The value to be written.
/// * `byte_buffer` - A mutable reference to a `BytesMut` buffer to which the value will be written.
pub fn write_long_signed(value: i64, byte_buffer: &mut BytesMut) {
    if value >= 0 {
        write_long_signed_pos(value as u64, byte_buffer);
    } else if value != i64::MIN {
        write_long_signed_neg(value, byte_buffer);
    } else {
        byte_buffer.extend_from_slice(&[0x7F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80]);
    }
}

//////////////////////////////////////////////////////////////////////////
// Read signed long using variable length encoding as defined in FAST spec
//////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Bytes, BytesMut};

    fn encode_decode_unsigned_test(value: u64) {
        let mut buffer = BytesMut::with_capacity(16);
        write_long_unsigned(value, &mut buffer);
        let encoded_bytes = buffer.freeze();
        let mut read_buffer = Bytes::copy_from_slice(&encoded_bytes);
        let decoded = read_long_unsigned(&mut read_buffer);

        assert_eq!(value, decoded.unwrap_or(if value == 0 { 1 } else { 0 }), "Failed at value: {:?} vs {:?}", value, decoded);
    }

    #[test]
    fn test_unsigned_boundaries() {
        encode_decode_unsigned_test(0);
        encode_decode_unsigned_test(1);
        encode_decode_unsigned_test(0x3F); // Boundary for 1-byte encoding
        encode_decode_unsigned_test(0x40); // Boundary for 2-byte encoding
        encode_decode_unsigned_test(0x2000); // 3-byte encoding
        encode_decode_unsigned_test(0x100000); // 4-byte
        encode_decode_unsigned_test(0x8000000); // 5-byte
        encode_decode_unsigned_test(u64::MAX - 2);
        encode_decode_unsigned_test(u64::MAX - 1);
        encode_decode_unsigned_test(u64::MAX);
    }

    fn encode_decode_signed_test(value: i64) {
        let mut buffer = BytesMut::with_capacity(16);
        write_long_signed(value, &mut buffer);
        let encoded_bytes = buffer.freeze();
        let mut read_buffer = Bytes::copy_from_slice(&encoded_bytes);
        let decoded = read_long_signed(&mut read_buffer);

        assert_eq!(value, decoded.unwrap_or(if value == 0 { 1 } else { 0 }), "Failed at value: {:?} vs {:?}", value, decoded);
    }

    #[test]
    fn test_signed_boundaries() {
        // Test common boundary values
        encode_decode_signed_test(0);
        encode_decode_signed_test(1);
        encode_decode_signed_test(i64::MAX);
        encode_decode_signed_test(i64::MAX - 1);
        encode_decode_signed_test(0x3F); // Boundary for 1-byte encoding
        encode_decode_signed_test(0x40); // Boundary for 2-byte encoding
        encode_decode_signed_test(0x2000); // 3-byte encoding
        encode_decode_signed_test(0x100000); // 4-byte
        encode_decode_signed_test(0x8000000); // 5-byte
        encode_decode_signed_test(i64::MIN / 2);
        encode_decode_signed_test(-9223372036854775808);
        encode_decode_signed_test(-4223372036854775808);
        encode_decode_signed_test(-5223372036854775808);
        encode_decode_signed_test(i64::MIN + 1);
        encode_decode_signed_test(i64::MIN);
        encode_decode_signed_test(-1); // ->  &[0xFF, 0x00];
        encode_decode_signed_test(-0x40); // Negative boundary for 1-byte encoding
        encode_decode_signed_test(-0x41); // Negative boundary for 2-byte encoding
        encode_decode_signed_test(-0x2001); // 3-byte encoding
        encode_decode_signed_test(-0x100001); // 4-byte
        encode_decode_signed_test(-0x8000001); // 5-byte
    }
}
