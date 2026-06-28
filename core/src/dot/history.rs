// ss[related telemetry.dot-export]
use log::*;
use std::fmt::Write;
use std::fs::{OpenOptions, create_dir_all};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::BytesMut;
use time::OffsetDateTime;
use time::macros::format_description;

use crate::core_exec;
use crate::monitor::ChannelMetaData;
use crate::serialize::byte_buffer_packer::PackedVecWriter;
use crate::serialize::fast_protocol_packed::write_long_unsigned;
use crate::telemetry::metrics_server;

pub(crate) const REC_NODE: u64 = 1;
pub(crate) const REC_EDGE: u64 = 0;
pub(crate) const HISTORY_WRITE_BLOCK_SIZE: usize = 1 << (12 + 4); // Must be power of 2 and 4096 or larger, 64k is good

/// Represents the frame history for a graph, including packed data and output paths.
pub struct FrameHistory {
    pub(crate) packed_sent_writer: PackedVecWriter<i64>,
    pub(crate) packed_take_writer: PackedVecWriter<i64>,
    pub(crate) history_buffer: BytesMut,
    pub(crate) guid: String,
    output_log_path: PathBuf,
    file_bytes_written: Arc<AtomicUsize>,
    last_file_to_append_onto: String,
    pub(crate) buffer_bytes_count: usize,
    local_thread_bytes_cache: usize,
}

impl FrameHistory {
    /// Creates a new `FrameHistory` instance.
    ///
    /// # Returns
    ///
    /// A new `FrameHistory` instance.
    pub fn new(ms_rate: u64) -> FrameHistory {
        let mut buf = BytesMut::with_capacity(HISTORY_WRITE_BLOCK_SIZE * 2);

        //set history file header with key information about our run
        //
        //what time did we start
        let now: u64 = OffsetDateTime::now_utc().unix_timestamp() as u64;
        write_long_unsigned(now, &mut buf);
        //
        //set history file header with key information about our frame rate
        write_long_unsigned(ms_rate, &mut buf); //time between frames

        let mut runtime_config = 0;

        #[cfg(test)]
        {
            runtime_config |= 1;
        } // ones bit is ether test(1) or release(0)

        #[cfg(feature = "prometheus_metrics")]
        {
            runtime_config |= 2;
        } // twos bit is ether prometheus(1) or none(0)

        #[cfg(feature = "proactor_tokio")]
        {
            runtime_config |= 4;
        } // fours bit is ether tokio(1) or nuclei(0)

        #[cfg(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))]
        {
            runtime_config |= 8;
        } // eights bit is ether telemetry(1) or none(0)

        #[cfg(feature = "telemetry_server_cdn")]
        {
            runtime_config |= 16;
        } // sixteenth bit is ether cdn(1) or builtin(0)

        //write bits which captured the runtime conditions
        write_long_unsigned(runtime_config, &mut buf);

        //TODO: add file version!!

        let result = FrameHistory {
            packed_sent_writer: PackedVecWriter::new(),
            packed_take_writer: PackedVecWriter::new(),
            history_buffer: buf,
            // Immutable details
            guid: uuid::Uuid::new_v4().to_string(), // Unique GUID for the run instance
            output_log_path: PathBuf::from("../output_logs"),
            // Running state
            file_bytes_written: Arc::new(AtomicUsize::from(0usize)),

            last_file_to_append_onto: "".to_string(),
            buffer_bytes_count: 0usize,
            local_thread_bytes_cache: 0usize,
        };

        let _ = create_dir_all(&result.output_log_path);
        result
    }

    /// Marks the current position in the history buffer.
    pub fn mark_position(&mut self) {
        self.buffer_bytes_count = self.history_buffer.len();
    }

    /// Applies a node definition to the history buffer.
    ///
    /// # Arguments
    ///
    /// * `name` - THE name of the node.
    /// * `id` - THE ID of the node.
    /// * `chin` - THE input channels.
    /// * `chout` - THE output channels.
    pub fn apply_node(
        &mut self,
        name: &'static str,
        id: usize,
        chin: &[Arc<ChannelMetaData>],
        chout: &[Arc<ChannelMetaData>],
    ) {
        write_long_unsigned(REC_NODE, &mut self.history_buffer); // Message type
        write_long_unsigned(id as u64, &mut self.history_buffer); // Message type

        write_long_unsigned(name.len() as u64, &mut self.history_buffer);
        self.history_buffer
            .write_str(name)
            .expect("internal error writing to ByteMut");

        write_long_unsigned(chin.len() as u64, &mut self.history_buffer);
        chin.iter().for_each(|meta| {
            write_long_unsigned(meta.id as u64, &mut self.history_buffer);
            write_long_unsigned(meta.labels.len() as u64, &mut self.history_buffer);
            meta.labels.iter().for_each(|s| {
                write_long_unsigned(s.len() as u64, &mut self.history_buffer);
                self.history_buffer
                    .write_str(s)
                    .expect("internal error writing to ByteMut");
            });
        });

        write_long_unsigned(chout.len() as u64, &mut self.history_buffer);
        chout.iter().for_each(|meta| {
            write_long_unsigned(meta.id as u64, &mut self.history_buffer);
            write_long_unsigned(meta.labels.len() as u64, &mut self.history_buffer);
            meta.labels.iter().for_each(|s| {
                write_long_unsigned(s.len() as u64, &mut self.history_buffer);
                self.history_buffer
                    .write_str(s)
                    .expect("internal error writing to ByteMut");
            });
        });
    }

    /// Applies an edge definition to the history buffer.
    ///
    /// # Arguments
    ///
    /// * `total_take_send` - THE total take and send values.
    /// * `frame_rate_ms` - THE frame rate in milliseconds.
    pub fn apply_edge(&mut self, total_take_send: &[(i64, i64)], frame_rate_ms: u64) {
        write_long_unsigned(REC_EDGE, &mut self.history_buffer); // Message type

        let total_take: Vec<i64> = total_take_send.iter().map(|(t, _)| *t).collect();
        let total_send: Vec<i64> = total_take_send.iter().map(|(_, s)| *s).collect();

        if (self.packed_sent_writer.delta_write_count() * frame_rate_ms as usize) < (10 * 60 * 1000)
        {
            self.packed_sent_writer
                .add_vec(&mut self.history_buffer, &total_send);
        } else {
            self.packed_sent_writer.sync_data();
            self.packed_sent_writer
                .add_vec(&mut self.history_buffer, &total_send);
        };

        if (self.packed_take_writer.delta_write_count() * frame_rate_ms as usize) < (10 * 60 * 1000)
        {
            self.packed_take_writer
                .add_vec(&mut self.history_buffer, &total_take);
        } else {
            self.packed_take_writer.sync_data();
            self.packed_take_writer
                .add_vec(&mut self.history_buffer, &total_take);
        };
    }

    /// Updates the history buffer, writing to disk if necessary.
    ///
    /// # Arguments
    ///
    /// * `flush_all` - A boolean indicating if all data should be flushed to disk.
    pub async fn update(&mut self, flush_all: bool) {
        // We write to disk in blocks just under a fixed power of two size
        // If we are about to enter a new block ensure we write the old one
        // NOTE: We block and do not write if the previous write was not completed.
        let cur_bytes_written = self.file_bytes_written.load(Ordering::SeqCst);

        if (flush_all || self.will_span_into_next_block())
            && (cur_bytes_written != self.local_thread_bytes_cache || 0 == cur_bytes_written)
        {
            // Store this and do not run again until this has changed
            self.local_thread_bytes_cache = cur_bytes_written;

            let buf_bytes_count = self.buffer_bytes_count;
            let continued_buffer: BytesMut = self.history_buffer.split_off(buf_bytes_count);
            // trace!("attempt to write history");
            let to_be_written =
                std::mem::replace(&mut self.history_buffer, continued_buffer).to_owned();
            if !to_be_written.is_empty() {
                let path = self.build_history_path().to_owned();

                let ptw = self.packed_take_writer.sync_required.clone();
                let psw = self.packed_sent_writer.sync_required.clone();
                let fbw = self.file_bytes_written.clone();

                // Let the file write happen in the background so we can get back to data updates
                // This is not a new thread so it is lightweight
                //TODO: rewrite as a new actor!
                core_exec::spawn_detached(async move {
                    if let Err(e) = Self::append_to_file(path, to_be_written, flush_all).await {
                        error!("Error writing to file: {}", e);
                        error!("Due to the above error some history has been lost");
                        // We force a full write for the next time around
                        ptw.store(true, Ordering::SeqCst);
                        psw.store(true, Ordering::SeqCst);
                    }
                    // Change the file_bytes_written to allow for the next spawn.
                    fbw.fetch_add(buf_bytes_count, Ordering::SeqCst);
                });
            }
        }
    }

    /// Builds the history file path based on the current date and GUID.
    ///
    /// # Returns
    ///
    /// THE history file path.
    pub(crate) fn build_history_path(&mut self) -> PathBuf {
        let format = format_description!("[year]_[month]_[day]");
        let log_time = OffsetDateTime::now_utc();
        let file_to_append_onto = format!(
            "{}_{}_log.dat",
            log_time
                .format(&format)
                .unwrap_or_else(|_| "0000_00_00".to_string()),
            self.guid
        );

        // If we are starting a new file reset our counter to zero
        if !self.last_file_to_append_onto.eq(&file_to_append_onto) {
            self.file_bytes_written.store(0, Ordering::SeqCst);
            self.last_file_to_append_onto
                .clone_from(&file_to_append_onto);
        }
        self.output_log_path.join(&file_to_append_onto)
    }

    /// Checks if the next block will span into the next file write block.
    ///
    /// # Returns
    ///
    /// `true` if the next block will span into the next file write block, `false` otherwise.
    fn will_span_into_next_block(&self) -> bool {
        let old_blocks = (self.file_bytes_written.load(Ordering::SeqCst) + self.buffer_bytes_count)
            / HISTORY_WRITE_BLOCK_SIZE;
        let new_blocks = (self.file_bytes_written.load(Ordering::SeqCst)
            + self.history_buffer.len())
            / HISTORY_WRITE_BLOCK_SIZE;
        new_blocks > old_blocks
    }

    /// Truncates the file at the given path and writes the provided data to it.
    ///
    /// # Arguments
    ///
    /// * `path` - THE file path.
    /// * `data` - THE data to write.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn truncate_file(path: PathBuf, data: BytesMut) -> Result<(), std::io::Error> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        metrics_server::async_write_all(data, false, file).await
    }

    async fn append_to_file(
        path: PathBuf,
        data: BytesMut,
        flush: bool,
    ) -> Result<(), std::io::Error> {
        let file = OpenOptions::new().append(true).create(true).open(&path)?;
        metrics_server::async_write_all(data, flush, file).await
    }
}
