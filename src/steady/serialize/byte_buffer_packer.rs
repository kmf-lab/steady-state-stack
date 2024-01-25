
use std::ops::{Add, Sub};
use bytes::{Bytes, BytesMut};
use num_traits::Zero;
use crate::steady::serialize::fast_protocol_packed::{read_long_signed, read_long_unsigned, write_long_signed, write_long_unsigned};


/// writes the deltas so we can write or send less data across the system.
/// note vecs must be the same size or larger never smaller as we go
pub(crate) struct PackedVecWriter<T> {
    pub(crate) previous: Vec<T>,
    pub(crate) write_count: usize,
    pub(crate) sync_required: bool, //next write is a full not a delta
}

impl<T> PackedVecWriter<T> {
    pub(crate) fn sync_data(&mut self) {
        self.sync_required = true;
    }
}

/// reads the deltas so we can write or send less data across the system.
impl <T> PackedVecWriter<T>
    where T: Sub<Output = T> + Into<i128> + Copy + Zero + PartialEq {

    fn consume_to_u64(bits:&Vec<u8>) -> Vec<u64> {
        let mut u64_values = Vec::with_capacity(1+(bits.len() >> 6));
        let mut p:usize = 0;
        while p<bits.len() {
            let mut value: u64 = 0;
            for i in 0..64 {
                if p < bits.len() {
                    value |= (bits[p] as u64) << i;
                    p += 1;
                } else {
                    break;
                }
            }
            u64_values.push(value);
        }
        u64_values
    }
    pub(crate) fn add_vec(&mut self, mut target: &mut BytesMut, source: &Vec<T>) {
        assert!(source.len() >= self.previous.len());

        if !self.sync_required {
            // which numbers changed? we use a 1 for them
            let zero = T::zero();
            let previous_iter = self.previous.iter().chain(std::iter::repeat(&zero));
            let mut bits: Vec<u8> = source.iter()
                .zip(previous_iter)
                .map(|(s, p)| (*s != *p) as u8)
                .collect();
            let mut chunks: Vec<u64> = Self::consume_to_u64(&bits);
            //remove any zeros off the end we do not need them
            while !chunks.is_empty() && 0 == chunks[chunks.len() - 1] {
                chunks.truncate(chunks.len() - 1);
            }
            //write length of chunks
            //NOTE below we write a negative length IFF we are sending a full record
            //     which means no bit mask and no deltas just each raw value
            write_long_signed(chunks.len() as i64, target);
            //write the bit mask for which do have changes
            chunks.iter().for_each(|c| write_long_unsigned(*c, target));
            //write each of the deltas
            self.previous.iter()
                .zip(source.iter())
                .for_each(|(p, s)| {
                    let dif: i128 = (*s - *p).into();
                    if 0 != dif {
                        write_long_signed(dif as i64, target);
                    }
                });
        } else {
            //negative length denotes we are sending a full record
            write_long_signed( -(source.len() as i64), target);
            source.iter().for_each(|s| write_long_signed((*s).into() as i64, target));
        };

        self.previous.clear();
        self.previous.extend_from_slice(source);
    }
}

/// reads the deltas so we can write or send less data across the system.
pub(crate) struct PackedVecReader<T> {
    pub(crate) previous: Vec<T>,
    pub(crate) write_count: usize,
}

impl <T> PackedVecReader<T>
    where T: From<i64> + Sub<Output = T> + Add<Output = T> + Copy {

    fn restore_vec(&mut self, buffer: &mut Bytes) -> Option<Vec<T>> {
        // Read the length of chunks
        let chunks_len = read_long_signed(buffer)?;

        let mut result: Vec<T> = Vec::with_capacity(self.previous.len());
        if chunks_len.is_positive() {


            // Read the bitmask chunks and reconstruct the bitmask
            let mut bitmask: Vec<u8> = Vec::new();
            for _ in 0..chunks_len {
                let chunk = read_long_unsigned(buffer)?;
                for i in 0..64 {
                    bitmask.push(((chunk >> i) & 1) as u8);
                }
            }

            // Apply differences based on the bitmask to reconstruct the vector
            let mut bitmask_iter = bitmask.into_iter();
            for &prev in &self.previous {
                if let Some(bit) = bitmask_iter.next() {
                    result.push(if bit.eq(&0u8) {
                        prev
                    } else {
                        let val: i64 = read_long_signed(buffer)?;
                        prev + T::from(val)
                    });
                } else { //if we have no more bits we have no more changes..
                    result.push(prev);
                }
            }
        } else {
            // Read the full vector
            for _ in 0..(-chunks_len) {
                result.push(T::from(read_long_signed(buffer)?));
            }
        }
        self.previous.clear();
        self.previous.extend_from_slice(&result);
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{BytesMut, Buf};



    #[test]
    fn test_round_trip_single_change() {
        let mut writer:PackedVecWriter<i128> = PackedVecWriter {
            previous: vec![1,2,3,4],
            sync_required: false,
            write_count: 0
        };
        let mut reader:PackedVecReader<i128> = PackedVecReader {
            previous: vec![1,2,3,4],
            write_count: 0
        };

        let mut buffer = BytesMut::new();
        let new_vec:Vec<i128> = vec![1, 2, 3, 5]; // One element changed
        writer.add_vec(&mut buffer, &new_vec);

        println!("buffer len {:?}",buffer.len());


        let mut buffer = buffer.freeze();
        let restored_vec = reader.restore_vec(&mut buffer);

        assert_eq!(restored_vec.unwrap(), new_vec);
    }

    #[test]
    fn test_round_trip_multiple_changes() {
        let mut writer:PackedVecWriter<i64> = PackedVecWriter {
            previous: vec![10, 20, 30, 40],
            sync_required: false,
            write_count: 0
        };
        let mut reader:PackedVecReader<i64> = PackedVecReader {
            previous: vec![10, 20, 30, 40],
            write_count: 0
        };

        let mut buffer = BytesMut::new();
        let new_vec = vec![11, 21, 31, 41]; // All elements changed
        writer.add_vec(&mut buffer, &new_vec);

        println!("buffer len {:?}",buffer.len());

        let mut buffer = buffer.freeze();
        let restored_vec = reader.restore_vec(&mut buffer);

        assert_eq!(restored_vec.unwrap(), new_vec);
    }

    #[test]
    fn test_round_trip_no_change() {
        let mut writer:PackedVecWriter<i128> = PackedVecWriter {
            previous: vec![5, 5, 5, 5],
            sync_required: true,
            write_count: 0
        };
        let mut reader:PackedVecReader<i128> = PackedVecReader {
            previous: vec![5, 5, 5, 5],
            write_count: 0
        };

        let mut buffer = BytesMut::new();
        let new_vec = vec![5, 5, 5, 5]; // No change
        writer.add_vec(&mut buffer, &new_vec);

        println!("buffer len {:?}",buffer.len());

        let mut buffer = buffer.freeze().clone();
        let restored_vec = reader.restore_vec(&mut buffer);

        assert_eq!(restored_vec.unwrap(), new_vec);
    }

    // Add more tests to cover different scenarios
}
