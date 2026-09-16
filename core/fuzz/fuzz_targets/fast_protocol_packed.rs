#![no_main]

//! Coverage-guided fuzz of FAST packed long encode/decode.

use bytes::{Bytes, BytesMut};
use libfuzzer_sys::fuzz_target;
use steady_state::fuzz_export::{
    read_long_signed, read_long_unsigned, write_long_signed, write_long_unsigned,
};

fuzz_target!(|data: &[u8]| {
    let slice = if data.len() > 64 { &data[..64] } else { data };

    let mut garbage = Bytes::copy_from_slice(slice);
    let _ = read_long_signed(&mut garbage);
    let mut garbage = Bytes::copy_from_slice(slice);
    let _ = read_long_unsigned(&mut garbage);

    if slice.len() >= 8 {
        let mut signed_bytes = [0u8; 8];
        signed_bytes.copy_from_slice(&slice[..8]);
        let signed = i64::from_le_bytes(signed_bytes);
        let mut enc = BytesMut::new();
        write_long_signed(signed, &mut enc);
        let mut dec = enc.freeze();
        match read_long_signed(&mut dec) {
            Some(got) => assert_eq!(got, signed, "signed FAST round-trip"),
            None => panic!("signed FAST encode must decode"),
        }

        let unsigned = u64::from_le_bytes(signed_bytes);
        let mut enc = BytesMut::new();
        write_long_unsigned(unsigned, &mut enc);
        let mut dec = enc.freeze();
        match read_long_unsigned(&mut dec) {
            Some(got) => assert_eq!(got, unsigned, "unsigned FAST round-trip"),
            None => panic!("unsigned FAST encode must decode"),
        }
    }
});
