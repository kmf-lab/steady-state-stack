
use std::ops::{Add, Sub};
use bytes::{Bytes, BytesMut};
use num_traits::Zero;
use crate::steady::serialize::fast_protocol_packed::{read_long_signed, read_long_unsigned, write_long_signed, write_long_unsigned};


/// writes the deltas so we can write or send less data across the system.
/// note vecs must be the same size or larger never smaller as we go
pub(crate) struct PackedVecWriter<T> {
    pub(crate) previous: Vec<T>,
    pub(crate) write_count: usize,
    pub(crate) delta_limit: usize, //after this many write a full record
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
        assert_eq!(true, source.len() >= self.previous.len());
        // which numbers changed? we use a 1 for them
        let zero = T::zero();
        let previous_iter = self.previous.iter().chain(std::iter::repeat(&zero));
        let mut bits:Vec<u8> = source.iter()
            .zip(previous_iter)
            .map(|(s, p)| (*s != *p) as u8)
            .collect();
        let mut chunks:Vec<u64> = Self::consume_to_u64(&bits);
        //remove any zeros off the end we do not need them
        while chunks.len()>0 && 0==chunks[chunks.len()-1] {
            chunks.truncate(chunks.len()-1);
        }
        //write length of chunks (yes unsigned would be better but not much)
        write_long_unsigned(chunks.len() as u64, &mut target);
        //write the bit mask for which do have changes
        chunks.iter().for_each(|c| write_long_unsigned(*c, &mut target));
        //write each of the deltas
        self.previous.iter()
            .zip(source.iter())
            .for_each(|(p,s)| {
                let dif:i128 = (*s - *p).into();
                if 0!=dif {
                    write_long_signed(dif as i64, &mut target);
                }
            });

        self.previous.clear();
        self.previous.extend_from_slice(source);
    }
}

/// reads the deltas so we can write or send less data across the system.
pub(crate) struct PackedVecReader<T> {
    pub(crate) previous: Vec<T>,
    pub(crate) write_count: usize,
    pub(crate) delta_limit: usize, //after this many write a full record
}

impl <T> PackedVecReader<T>
    where T: From<i64> + Sub<Output = T> + Add<Output = T> + Copy {

    fn restore_vec(&mut self, buffer: &mut Bytes) -> Option<Vec<T>> {
        // Read the length of chunks
        let chunks_len = read_long_unsigned(buffer)?;

        // Read the bitmask chunks and reconstruct the bitmask
        let mut bitmask: Vec<u8> = Vec::new();
        for _ in 0..chunks_len {
            let chunk = read_long_unsigned(buffer)?;
            for i in 0..64 {
                bitmask.push(((chunk >> i) & 1) as u8);
            }
        }

        // Apply differences based on the bitmask to reconstruct the vector
        let mut result: Vec<T> = Vec::with_capacity(self.previous.len());
        let mut bitmask_iter = bitmask.into_iter();
        for &prev in &self.previous {
            if let Some(bit) = bitmask_iter.next() {
                result.push( if bit.eq(&0u8) {
                    prev
                } else {
                    let val:i64 = read_long_signed(buffer)?;
                    prev + T::from(val)

                });
            } else { //if we have no more bits we have no more changes..
                result.push( prev );
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
            delta_limit: usize::MAX,
            write_count: 0
        };
        let mut reader:PackedVecReader<i128> = PackedVecReader {
            previous: vec![1,2,3,4],
            delta_limit: usize::MAX,
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
            delta_limit: usize::MAX,
            write_count: 0
        };
        let mut reader:PackedVecReader<i64> = PackedVecReader {
            previous: vec![10, 20, 30, 40],
            delta_limit: usize::MAX,
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
            delta_limit: usize::MAX,
            write_count: 0
        };
        let mut reader:PackedVecReader<i128> = PackedVecReader {
            previous: vec![5, 5, 5, 5],
            delta_limit: usize::MAX,
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
