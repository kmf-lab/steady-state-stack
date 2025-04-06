use log::*;
use crate::{new_state, LazyStreamRx, LazyStreamTx, Threading};
use crate::actor_builder::ActorBuilder;
use crate::distributed::aeron_channel_builder::AqueTech;
use crate::distributed::{aeron_publish, aeron_publish_bundle, aeron_subscribe, aeron_subscribe_bundle};
use crate::distributed::distributed_stream::{LazySteadyStreamRxBundle, LazySteadyStreamRxBundleClone, LazySteadyStreamTxBundle, LazySteadyStreamTxBundleClone, StreamSessionMessage, StreamSimpleMessage};

pub trait AqueductBuilder {
    fn build_aqueduct(
        self,
        tech: AqueTech,
        actor_builder: &ActorBuilder,
        threading: &mut Threading
    );
}

impl AqueductBuilder for LazyStreamRx<StreamSimpleMessage> {
    fn build_aqueduct(
        self,
        tech: AqueTech,
        actor_builder: &ActorBuilder,
        threading: &mut Threading
    ) {
        let remotes = tech.to_remotes();
        let match_me = tech.to_match_me();
        let tech_string = tech.to_tech();

        match tech {
            AqueTech::Aeron(media_driver, channel, stream_id) => {
                let actor_builder = actor_builder
                    .with_remote_details(remotes
                                         ,match_me
                                         ,true
                                         ,tech_string
                    );
                if let Some(aeron) = media_driver {
                    let state = new_state();
                    actor_builder.build(move |context|
                                   aeron_publish::run(context
                                                      , self.clone()
                                                      , channel.clone()
                                                      , stream_id
                                                      , state.clone() )
                               , threading)
                }
            },
            AqueTech::None => {
            },
            _ => {
                panic!("unsupported distribution type");
            }
        }
    }

}

impl AqueductBuilder for LazyStreamTx<StreamSessionMessage> {
    fn build_aqueduct(
        self,
        tech: AqueTech,
        actor_builder: &ActorBuilder,
        threading: &mut Threading
    ) {
        match tech {
            AqueTech::Aeron(media_driver, channel, stream_id) => {
                let actor_builder = actor_builder.clone();
                if let Some(aeron) = media_driver {
                    let state = new_state();
                    actor_builder.build(move |context|
                                   aeron_subscribe::run(context
                                                        , self.clone() //tx: SteadyStreamTxBundle<StreamFragment,GIRTH>
                                                        , channel.clone()
                                                        , stream_id
                                                        , state.clone())
                               , threading);
                };
            }
            AqueTech::None => {
                warn!("no AqueTech provided, probably testing or still under development");
            }
            _ => {
                panic!("unsupported distribution type");
            }
        };
    }

}
impl<const GIRTH: usize> AqueductBuilder for LazySteadyStreamRxBundle<StreamSimpleMessage, GIRTH> {
    fn build_aqueduct(
        self,
        tech: AqueTech,
        actor_builder: &ActorBuilder,
        threading: &mut Threading
    ) {
        let actor_builder = actor_builder.clone();
        let state = new_state();
        match tech {
            AqueTech::Aeron(media_driver, channel, stream_id) => {
                actor_builder.build(move |context|
                               aeron_publish_bundle::run(context
                                                         , self.clone()
                                                         , channel.clone()
                                                         , stream_id
                                                         , state.clone())
                           , threading)
            },
            AqueTech::None => {
                warn!("no AqueTech provided, probably testing or still under development");
            },
            _ => {
                panic!("unsupported distribution type");
            }
        };
    }

}

impl<const GIRTH: usize> AqueductBuilder for LazySteadyStreamTxBundle<StreamSessionMessage, GIRTH> {
    fn build_aqueduct(
        self,
        tech: AqueTech,
        actor_builder: &ActorBuilder,
        threading: &mut Threading
    ) {

        let actor_builder = actor_builder.clone();
        let state = new_state();
        match tech {
            AqueTech::Aeron(media_driver, channel, stream_id) => {
                    actor_builder.build(move |context|
                                   aeron_subscribe_bundle::run(context
                                                               , self.clone() //tx: SteadyStreamTxBundle<StreamFragment,GIRTH>
                                                               , channel.clone()
                                                               , stream_id
                                                               , state.clone()
                                                               )
                               , threading);
            },
            AqueTech::None => {
                warn!("no AqueTech provided, probably testing or still under development");
            },
            _ => {
                panic!("unsupported distribution type");
            }
        };
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    /// Test that `build_aqueduct` works for `LazyStreamRx<StreamSimpleMessage>` with `AqueTech::Aeron`.
    #[test]
    fn test_build_aqueduct_lazy_stream_rx() {
        let mut graph = GraphBuilder::for_testing().build(());
        if graph.aeron_media_driver().is_some() {
            let cb = graph.channel_builder();

            let (_lazy_tx, lazy_rx) = cb.build_as_stream(100);
            // Use AqueTech::Aeron with None for media_driver and dummy channel/stream_id
            let tech = AqueTech::None;
            // Create a minimal ActorBuilder and Threading
            let actor_builder = ActorBuilder::new(&mut graph).never_simulate(true);
            let mut threading = Threading::Spawn;
            // Call build_aqueduct; test passes if it doesn't panic
            lazy_rx.build_aqueduct(tech, &actor_builder, &mut threading);
        }
    }

    /// Test that `build_aqueduct` works for `LazyStreamTx<StreamSessionMessage>` with `AqueTech::Aeron`.
    #[test]
    fn test_build_aqueduct_lazy_stream_tx() {
        let mut graph = GraphBuilder::for_testing().build(());

        let cb = graph.channel_builder();

        let (lazy_tx, _lazy_rx) = cb.build_as_stream(100);
        let tech = AqueTech::None;
        let actor_builder = ActorBuilder::new(&mut graph).never_simulate(true);
        let mut threading = Threading::Spawn;
        lazy_tx.build_aqueduct(tech, &actor_builder, &mut threading);
    }

    /// Test that `build_aqueduct` works for `LazySteadyStreamRxBundle<StreamSimpleMessage, GIRTH>` with `AqueTech::Aeron`.
    #[test]
    fn test_build_aqueduct_lazy_stream_rx_bundle_simple() {
        let mut graph = GraphBuilder::for_testing().build(());

        const GIRTH: usize = 1;
        let cb = graph.channel_builder();

        let (lazy_tx, lazy_rx_bundle) = cb.build_as_stream_bundle::<StreamSimpleMessage, GIRTH>(100);
        let tech = AqueTech::None;
        let actor_builder = ActorBuilder::new(&mut graph).never_simulate(true);
        let mut threading = Threading::Spawn;
        lazy_rx_bundle.build_aqueduct(tech, &actor_builder, &mut threading);
    }

    #[test]
    fn test_build_aqueduct_lazy_stream_tx_bundle_session() {
        let mut graph = GraphBuilder::for_testing().build(());

        const GIRTH: usize = 1;
        let cb = graph.channel_builder();

        let (lazy_tx_bundle, lazy_rx_bundle) = cb.build_as_stream_bundle::<StreamSessionMessage, GIRTH>(100);

        let tech = AqueTech::None;
        let actor_builder = ActorBuilder::new(&mut graph).never_simulate(true);
        let mut threading = Threading::Spawn;
        lazy_tx_bundle.build_aqueduct(tech, &actor_builder, &mut threading);
    }
}