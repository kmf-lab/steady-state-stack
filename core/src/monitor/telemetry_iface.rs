// ss[related philosophy.single-wake-up]
use std::sync::Arc;

// ss[related philosophy.structural-hierarchy]
use crate::monitor::metadata::{ActorMetaData, ActorStatus, ChannelMetaData};
// ss[related philosophy.single-wake-up]
use crate::SteadyRx;

/// Defines methods for telemetry receivers to manage and access telemetry data.
///
/// This trait ensures that implementations are thread-safe and can be sent across threads.
// ss[related philosophy.single-wake-up]
pub trait RxTel: Send + Sync {
    /// Returns a vector of metadata for all transmitter channels.
    // ss[related philosophy.structural-hierarchy]
    fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;

    /// Returns a vector of metadata for all receiver channels.
    // ss[related philosophy.single-wake-up]
    fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;

    /// Consumes and returns the current actor status, if available.
    // ss[related philosophy.single-wake-up]
    fn consume_actor(&self) -> Option<ActorStatus>;

    /// Consumes a pending DOT graph subtitle update for this telemetry receiver, if any.
    ///
    /// Return value: `None` = no pending change; `Some(None)` = clear subtitle; `Some(Some(s))` =
    /// set subtitle to `s`. Default: no subtitle channel (always `None`).
    // ss[related philosophy.single-wake-up]
    fn consume_dot_subtitle(&self) -> Option<Option<String>> {
        None
    }

    /// Returns the metadata associated with the actor.
    // ss[related philosophy.single-wake-up]
    fn actor_metadata(&self) -> Arc<ActorMetaData>;

    /// Consumes take data into the provided vectors, indicating whether data was consumed.
    // ss[related philosophy.single-wake-up]
    fn consume_take_into(
        &self,
        take_send_source: &mut Vec<(i64, i64)>,
        future_take: &mut Vec<i64>,
        future_send: &mut Vec<i64>,
    ) -> bool;

    /// Consumes send data into the provided vectors, indicating whether data was consumed.
    // ss[related philosophy.single-wake-up]
    fn consume_send_into(
        &self,
        take_send_source: &mut Vec<(i64, i64)>,
        future_send: &mut Vec<i64>,
    ) -> bool;

    /// Returns an actor receiver definition for the specified version, if available.
    // ss[related philosophy.single-wake-up]
    fn actor_rx(&self, version: u32) -> Option<Box<SteadyRx<ActorStatus>>>;

    /// Checks if the telemetry is empty and the channel is closed.
    // ss[related philosophy.single-wake-up]
    fn is_empty_and_closed(&self) -> bool;

    /// Checks if the telemetry is currently empty.
    // ss[related philosophy.single-wake-up]
    fn is_empty(&self) -> bool;
}

#[cfg(test)]
// ss[related philosophy.single-wake-up]
mod tests {
    // ss[related philosophy.structural-hierarchy]
    use std::sync::Arc;

    // ss[related philosophy.single-wake-up]
    use super::RxTel;
    // ss[related philosophy.structural-hierarchy]
    use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData};
    // ss[related philosophy.structural-hierarchy]
    use crate::SteadyRx;

    /// Minimal `RxTel` stub that relies on the trait default for `consume_dot_subtitle`.
    // ss[related philosophy.single-wake-up]
    struct StubRxTel {
        actor_metadata: Arc<ActorMetaData>,
    }

    /// Stub that overrides `consume_dot_subtitle` to exercise the trait method path.
    // ss[related philosophy.single-wake-up]
    struct SubtitleRxTel {
        actor_metadata: Arc<ActorMetaData>,
        pending: Option<Option<String>>,
    }

    // ss[related philosophy.single-wake-up]
    impl RxTel for StubRxTel {
        // ss[related philosophy.structural-hierarchy]
        fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
            Vec::new()
        }

        // ss[related philosophy.single-wake-up]
        fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
            Vec::new()
        }

        // ss[related philosophy.single-wake-up]
        fn consume_actor(&self) -> Option<ActorStatus> {
            None
        }

        // ss[related philosophy.single-wake-up]
        fn actor_metadata(&self) -> Arc<ActorMetaData> {
            self.actor_metadata.clone()
        }

        // ss[related philosophy.single-wake-up]
        fn consume_take_into(
            &self,
            _take_send_source: &mut Vec<(i64, i64)>,
            _future_take: &mut Vec<i64>,
            _future_send: &mut Vec<i64>,
        ) -> bool {
            false
        }

        // ss[related philosophy.single-wake-up]
        fn consume_send_into(
            &self,
            _take_send_source: &mut Vec<(i64, i64)>,
            _future_send: &mut Vec<i64>,
        ) -> bool {
            false
        }

        // ss[related philosophy.single-wake-up]
        fn actor_rx(&self, _version: u32) -> Option<Box<SteadyRx<ActorStatus>>> {
            None
        }

        // ss[related philosophy.single-wake-up]
        fn is_empty_and_closed(&self) -> bool {
            true
        }

        // ss[related philosophy.single-wake-up]
        fn is_empty(&self) -> bool {
            true
        }
    }

    // ss[related philosophy.single-wake-up]
    impl RxTel for SubtitleRxTel {
        // ss[related philosophy.structural-hierarchy]
        fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
            Vec::new()
        }

        // ss[related philosophy.single-wake-up]
        fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
            Vec::new()
        }

        // ss[related philosophy.single-wake-up]
        fn consume_actor(&self) -> Option<ActorStatus> {
            None
        }

        // ss[related philosophy.single-wake-up]
        fn consume_dot_subtitle(&self) -> Option<Option<String>> {
            self.pending.clone()
        }

        // ss[related philosophy.single-wake-up]
        fn actor_metadata(&self) -> Arc<ActorMetaData> {
            self.actor_metadata.clone()
        }

        // ss[related philosophy.single-wake-up]
        fn consume_take_into(
            &self,
            _take_send_source: &mut Vec<(i64, i64)>,
            _future_take: &mut Vec<i64>,
            _future_send: &mut Vec<i64>,
        ) -> bool {
            false
        }

        // ss[related philosophy.single-wake-up]
        fn consume_send_into(
            &self,
            _take_send_source: &mut Vec<(i64, i64)>,
            _future_send: &mut Vec<i64>,
        ) -> bool {
            false
        }

        // ss[related philosophy.single-wake-up]
        fn actor_rx(&self, _version: u32) -> Option<Box<SteadyRx<ActorStatus>>> {
            None
        }

        // ss[related philosophy.single-wake-up]
        fn is_empty_and_closed(&self) -> bool {
            true
        }

        // ss[related philosophy.single-wake-up]
        fn is_empty(&self) -> bool {
            true
        }
    }

    #[test]
    // ss[verify philosophy.single-wake-up]
    fn consume_dot_subtitle_default_returns_none() {
        let stub = StubRxTel {
            actor_metadata: Arc::new(ActorMetaData::default()),
        };
        assert_eq!(stub.consume_dot_subtitle(), None);
        assert!(stub.is_empty());
        assert!(stub.is_empty_and_closed());
        assert!(stub.tx_channel_id_vec().is_empty());
        assert!(stub.rx_channel_id_vec().is_empty());
        assert!(stub.consume_actor().is_none());
        assert!(stub.actor_rx(0).is_none());
    }

    #[test]
    // ss[verify philosophy.single-wake-up]
    fn consume_dot_subtitle_override_returns_pending_value() {
        let stub = SubtitleRxTel {
            actor_metadata: Arc::new(ActorMetaData::default()),
            pending: Some(Some("subtitle".into())),
        };
        assert_eq!(
            stub.consume_dot_subtitle(),
            Some(Some("subtitle".into()))
        );
        let clear = SubtitleRxTel {
            actor_metadata: Arc::new(ActorMetaData::default()),
            pending: Some(None),
        };
        assert_eq!(clear.consume_dot_subtitle(), Some(None));
    }

    #[test]
    // ss[verify philosophy.single-wake-up]
    fn stub_consume_into_methods_return_false_without_mutation() {
        let stub = StubRxTel {
            actor_metadata: Arc::new(ActorMetaData::default()),
        };
        let mut take_send = vec![(1, 10)];
        let mut future_take = vec![5];
        let mut future_send = vec![0];
        assert!(!stub.consume_take_into(&mut take_send, &mut future_take, &mut future_send));
        assert_eq!(take_send, vec![(1, 10)]);

        let mut send_target = vec![(0, 100)];
        let mut pending_send = vec![7];
        assert!(!stub.consume_send_into(&mut send_target, &mut pending_send));
        assert_eq!(send_target, vec![(0, 100)]);
        assert_eq!(pending_send, vec![7]);
    }
}
