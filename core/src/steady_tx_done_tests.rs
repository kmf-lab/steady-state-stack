// ss[related philosophy.structural-hierarchy]
use super::*;

#[test]
// ss[verify channel.backpressure-never-drop]
fn test_tx_done_payload_count_normal() {
    let done = TxDone::Normal(5);
    assert_eq!(done.item_count(), 5);
    assert_eq!(done.payload_count(), None);
}

#[test]
// ss[verify channel.backpressure-never-drop]
fn test_tx_done_payload_count_stream() {
    let done = TxDone::Stream(3, 100);
    assert_eq!(done.item_count(), 3);
    assert_eq!(done.payload_count(), Some(100));
}

#[test]
// ss[verify channel.backpressure-never-drop]
fn test_tx_done_deref() {
    let normal: usize = *TxDone::Normal(7);
    assert_eq!(normal, 7);
    let stream: usize = *TxDone::Stream(2, 200);
    assert_eq!(stream, 2);
}
