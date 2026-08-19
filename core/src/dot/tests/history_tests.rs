// ss[related telemetry.dot-export]
use super::super::*;
// ss[impl telemetry.dot-export]
use super::test_dot_frames;
// ss[impl telemetry.dot-export]
use crate::dot_unify::ChannelEdgeRole;
// ss[related telemetry.dot-export]
use crate::monitor::{ActorIdentity, ActorMetaData, ActorStatus, ChannelMetaData};
// ss[impl telemetry.dot-export]
use crate::telemetry::metrics_server::async_write_all;
// ss[impl telemetry.dot-export]
use bytes::BytesMut;
// ss[related telemetry.dot-export]
use std::fs::remove_file;
// ss[impl telemetry.dot-export]
use std::path::PathBuf;
// ss[impl telemetry.dot-export]
use std::sync::Arc;
// ss[related telemetry.dot-export]
use std::time::Instant;

#[test]
// ss[verify telemetry.dot-export]
fn test_frame_history_new() {
    let frame_history = FrameHistory::new(1000);
    assert_eq!(frame_history.packed_sent_writer.delta_write_count(), 0);
    assert_eq!(frame_history.packed_take_writer.delta_write_count(), 0);
    assert!(!frame_history.history_buffer.is_empty());
}


#[test]
// ss[verify telemetry.dot-export]
fn test_frame_history_apply_node() {
    let mut frame_history = FrameHistory::new(1000);
    let chin = vec![Arc::new(ChannelMetaData::default())];
    let chout = vec![Arc::new(ChannelMetaData::default())];
    frame_history.apply_node("node1", 1, &chin, &chout);
    assert!(!frame_history.history_buffer.is_empty());
}


#[test]
// ss[verify telemetry.dot-export]
fn test_frame_history_apply_edge() {
    let mut frame_history = FrameHistory::new(1000);
    let total_take_send = vec![(100, 50)];
    frame_history.apply_edge(&total_take_send, 1000);
    assert!(!frame_history.history_buffer.is_empty());
}


#[test]
// ss[verify telemetry.dot-export]
fn test_frame_history_mark_position() {
    let mut frame_history = FrameHistory::new(1000);
    frame_history.mark_position();
    assert_eq!(
        frame_history.buffer_bytes_count,
        frame_history.history_buffer.len()
    );
}


#[test]
// ss[verify telemetry.dot-export]
fn test_frame_history_update() {
    let mut frame_history = FrameHistory::new(1000);
    frame_history.mark_position();

    core_exec::block_on(frame_history.update(true));

    assert_eq!(frame_history.history_buffer.len(), 0);
}


#[test]
// ss[verify telemetry.dot-export]
fn test_frame_history_all_to_file_async() {
    let data = BytesMut::from("test data");
    let path = PathBuf::from("test_all_to_file.dat");
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .open(&path)
        .expect("Failed to open file");

    let _ = core_exec::block_on(async_write_all(data, true, file));

    let result = std::fs::read_to_string(&path).expect("Failed to read written file");
    assert_eq!(result, "test data");

    // Clean up
    let _ = remove_file(&path);
}

