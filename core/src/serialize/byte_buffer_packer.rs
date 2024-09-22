use std::ops::{Add, Sub};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use bytes::{Bytes, BytesMut};
use num_traits::Zero;
use crate::serialize::fast_protocol_packed::{read_long_signed, read_long_unsigned, write_long_signed, write_long_unsigned};

/// PackedVecWriter is a helper class for writing history data.
/// It writes the deltas (differences) between consecutive data points to minimize the amount of data written or sent across the system.
///
/// # Generics
///
/// * `T` - The type of the elements in the vector. It must implement the `Sub`, `Into<i128>`, `Copy`, `Zero`, and `PartialEq` traits.
pub(crate) struct PackedVecWriter<T> {
    pub(crate) previous: Vec<T>,
    pub(crate) delta_write_count: usize,
    pub(crate) sync_required: Arc<AtomicBool>, // Indicates if the next write should be a full sync rather than a delta
}

impl<T> PackedVecWriter<T> {
    /// Creates a new `PackedVecWriter` instance.
    pub fn new() -> Self {
        PackedVecWriter {
            previous: Vec::new(),
            delta_write_count: 0,
            sync_required: Arc::new(AtomicBool::from(true)),
        }
    }

    /// Marks the data as requiring a full sync on the next write.
    pub fn sync_data(&mut self) {
        self.sync_required = Arc::new(AtomicBool::from(true));
    }

    /// Returns the count of delta writes performed.
    pub fn delta_write_count(&self) -> usize {
        self.delta_write_count
    }
}

/// Implements the functionality to add vectors and compute deltas.
///
/// # Generics
///
/// * `T` - The type of the elements in the vector. It must implement the `Sub`, `Into<i128>`, `Copy`, `Zero`, and `PartialEq` traits.
impl<T> PackedVecWriter<T>
    where
        T: Sub<Output = T> + Into<i128> + Copy + Zero + PartialEq,
{
    /// Converts a slice of bits into a vector of `u64` values.
    ///
    /// # Arguments
    ///
    /// * `bits` - A slice of bits.
    ///
    /// # Returns
    ///
    /// A vector of `u64` values.
    fn consume_to_u64(bits: &[u8]) -> Vec<u64> {
        let mut u64_values = Vec::with_capacity(1 + (bits.len() >> 6));
        let mut p: usize = 0;
        while p < bits.len() {
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

    /// Adds a vector to the writer and computes the deltas.
    ///
    /// # Arguments
    ///
    /// * `target` - A mutable reference to the `BytesMut` buffer where the data will be written.
    /// * `source` - A slice of the source vector.
    ///
    /// # Panics
    ///
    /// Panics if the length of the source vector is smaller than the length of the previous vector.
    pub(crate) fn add_vec(&mut self, target: &mut BytesMut, source: &[T]) {
        assert!(
            source.len() >= self.previous.len(),
            "new source {:?} >= prev {:?}",
            source.len(),
            self.previous.len()
        );

        if !self.sync_required.load(Ordering::SeqCst) {
            // Compute which numbers changed (1 if changed, 0 otherwise)
            let zero = T::zero();
            let previous_iter = self.previous.iter().chain(std::iter::repeat(&zero));
            let bits: Vec<u8> = source.iter()
                .zip(previous_iter)
                .map(|(s, p)| (*s != *p) as u8)
                .collect();
            let mut chunks: Vec<u64> = Self::consume_to_u64(&bits);

            // Remove any trailing zeros
            while !chunks.is_empty() && 0 == chunks[chunks.len() - 1] {
                chunks.truncate(chunks.len() - 1);
            }

            // Write length of chunks
            // Negative length indicates a full record
            write_long_signed(chunks.len() as i64, target);

            // Write the bit mask for which elements have changes
            chunks.iter().for_each(|c| write_long_unsigned(*c, target));

            // Write each of the deltas
            self.previous.iter()
                .zip(source.iter())
                .for_each(|(p, s)| {
                    let dif: i128 = (*s - *p).into();
                    if dif != 0 {
                        write_long_signed(dif as i64, target);
                    }
                });

            self.delta_write_count += 1;
        } else {
            // Negative length denotes a full record
            write_long_signed(-(source.len() as i64), target);
            source.iter().for_each(|s| write_long_signed((*s).into() as i64, target));

            self.delta_write_count = 0;
            self.sync_required = Arc::new(AtomicBool::from(false));
        };

        self.previous.clear();
        self.previous.extend_from_slice(source);
    }
}

/// PackedVecReader is a helper class for reading history data.
/// It reads the deltas (differences) between consecutive data points to reconstruct the original data.
///
/// # Generics
///
/// * `T` - The type of the elements in the vector. It must implement the `Sub`, `Add`, `Copy`, and `From<i64>` traits.
#[allow(dead_code)]
pub(crate) struct PackedVecReader<T> {
    pub(crate) previous: Vec<T>,
}

impl<T> PackedVecReader<T>
    where
        T: From<i64> + Sub<Output = T> + Add<Output = T> + Copy,
{
    /// Restores a vector from the buffer by applying the deltas.
    ///
    /// # Arguments
    ///
    /// * `buffer` - A mutable reference to the `Bytes` buffer containing the data.
    ///
    /// # Returns
    ///
    /// An `Option` containing the restored vector or `None` if the read failed.
    #[allow(dead_code)]
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
                    result.push(if bit == 0 {
                        prev
                    } else {
                        let val: i64 = read_long_signed(buffer)?;
                        prev + T::from(val)
                    });
                } else {
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
    use bytes::BytesMut;
    use log::trace;

    #[test]
    fn test_round_trip_single_change() {
        let mut writer: PackedVecWriter<i128> = PackedVecWriter {
            previous: vec![1, 2, 3, 4],
            sync_required: Arc::new(AtomicBool::from(false)),
            delta_write_count: 0,
        };
        let mut reader: PackedVecReader<i128> = PackedVecReader {
            previous: vec![1, 2, 3, 4],
        };

        let mut buffer = BytesMut::new();
        let new_vec: Vec<i128> = vec![1, 2, 3, 5]; // One element changed
        writer.add_vec(&mut buffer, &new_vec);

        trace!("buffer len {:?}", buffer.len());

        let mut buffer = buffer.freeze();
        let restored_vec = reader.restore_vec(&mut buffer);

        assert_eq!(restored_vec.unwrap(), new_vec);
    }

    #[test]
    fn test_round_trip_multiple_changes() {
        let mut writer: PackedVecWriter<i64> = PackedVecWriter {
            previous: vec![10, 20, 30, 40],
            sync_required: Arc::new(AtomicBool::from(false)),
            delta_write_count: 0,
        };
        let mut reader: PackedVecReader<i64> = PackedVecReader {
            previous: vec![10, 20, 30, 40],
        };

        let mut buffer = BytesMut::new();
        let new_vec = vec![11, 21, 31, 41]; // All elements changed
        writer.add_vec(&mut buffer, &new_vec);

        trace!("buffer len {:?}", buffer.len());

        let mut buffer = buffer.freeze();
        let restored_vec = reader.restore_vec(&mut buffer);

        assert_eq!(restored_vec.unwrap(), new_vec);
    }

    #[test]
    fn test_round_trip_no_change() {
        let mut writer: PackedVecWriter<i128> = PackedVecWriter {
            previous: vec![5, 5, 5, 5],
            sync_required: Arc::new(AtomicBool::from(true)),
            delta_write_count: 0,
        };
        let mut reader: PackedVecReader<i128> = PackedVecReader {
            previous: vec![5, 5, 5, 5],
        };

        let mut buffer = BytesMut::new();
        let new_vec = vec![5, 5, 5, 5]; // No change
        writer.add_vec(&mut buffer, &new_vec);

        trace!("buffer len {:?}", buffer.len());

        let mut buffer = buffer.freeze().clone();
        let restored_vec = reader.restore_vec(&mut buffer);

        assert_eq!(restored_vec.unwrap(), new_vec);
    }

    // Add more tests to cover different scenarios
}
