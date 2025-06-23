#[allow(unused_imports)]
use log::*;
use crate::{new_state, LazyStreamRx, LazyStreamTx, ScheduleAs};
use crate::actor_builder::ActorBuilder;
use crate::distributed::aeron_channel_builder::AqueTech;
use crate::distributed::{aeron_publish, aeron_publish_bundle, aeron_subscribe, aeron_subscribe_bundle};
use crate::distributed::distributed_stream::{LazySteadyStreamRxBundle, LazySteadyStreamRxBundleClone, LazySteadyStreamTxBundle, LazySteadyStreamTxBundleClone, StreamIngress, StreamEgress};

pub trait AqueductBuilder {
    fn build_aqueduct(
        self,
        tech: AqueTech,
        actor_builder: &ActorBuilder,
        threading: ScheduleAs
    );
}

impl AqueductBuilder for LazyStreamRx<StreamEgress> {
    fn build_aqueduct(
        self,
        tech: AqueTech,
        actor_builder: &ActorBuilder,
        threading: ScheduleAs
    ) {
        let remotes = tech.to_remotes();
        let match_me = tech.to_match_me();
        let tech_string = tech.to_tech();

        match tech {
            AqueTech::Aeron(channel, stream_id) => {
                let actor_builder = actor_builder
                    .with_remote_details(remotes
                                         ,match_me
                                         ,true
                                         ,tech_string
                    );
                let state = new_state();
                actor_builder.build(move |context|
                               aeron_publish::run(context
                                                  , self.clone()
                                                  , channel.clone()
                                                  , stream_id
                                                  , state.clone() )
                           , threading)

            },
            AqueTech::None => {
            },
            // _ => {
            //     panic!("unsupported distribution type");
            // }
        }
    }

}

impl AqueductBuilder for LazyStreamTx<StreamIngress> {
    fn build_aqueduct(
        self,
        tech: AqueTech,
        actor_builder: &ActorBuilder,
        threading: ScheduleAs
    ) {

        let remotes = tech.to_remotes();
        let match_me = tech.to_match_me();
        let tech_string = tech.to_tech();

        match tech {
            AqueTech::Aeron(channel, stream_id) => {
                let actor_builder = actor_builder
                    .with_remote_details(remotes
                                         ,match_me
                                         ,false
                                         ,tech_string
                    );
                let state = new_state();
                actor_builder.build(move |context|
                               aeron_subscribe::run(context
                                                    , self.clone() //tx: SteadyStreamTxBundle<StreamFragment,GIRTH>
                                                    , channel.clone()
                                                    , stream_id
                                                    , state.clone())
                           , threading);

            }
            AqueTech::None => {
                #[cfg(not(test))]
                warn!("no AqueTech provided, probably testing or still under development");
            }
        };
    }

}
impl<const GIRTH: usize> AqueductBuilder for LazySteadyStreamRxBundle<StreamEgress, GIRTH> {
    fn build_aqueduct(
        self,
        tech: AqueTech,
        actor_builder: &ActorBuilder,
        threading: ScheduleAs
    ) {
        let remotes = tech.to_remotes();
        let match_me = tech.to_match_me();
        let tech_string = tech.to_tech();

        let actor_builder = actor_builder
            .with_remote_details(remotes
                                 ,match_me
                                 ,true
                                 ,tech_string
            );
        let state = new_state();
        match tech {
            AqueTech::Aeron(channel, stream_id) => {
                actor_builder.build(move |context|
                               aeron_publish_bundle::run(context
                                                         , self.clone()
                                                         , channel.clone()
                                                         , stream_id
                                                         , state.clone())
                           , threading)
            },
            AqueTech::None => {
                #[cfg(not(test))]
                warn!("no AqueTech provided, probably testing or still under development");
            },
        };
    }

}

impl<const GIRTH: usize> AqueductBuilder for LazySteadyStreamTxBundle<StreamIngress, GIRTH> {
    fn build_aqueduct(
        self,
        tech: AqueTech,
        actor_builder: &ActorBuilder,
        threading: ScheduleAs
    ) {
        let remotes = tech.to_remotes();
        let match_me = tech.to_match_me();
        let tech_string = tech.to_tech();

        let actor_builder = actor_builder
            .with_remote_details(remotes
                                 ,match_me
                                 ,false
                                 ,tech_string
            );
        let state = new_state();
        match tech {
            AqueTech::Aeron(channel, stream_id) => {
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
                #[cfg(not(test))]
                warn!("no AqueTech provided, probably testing or still under development");
            },
        };
    }

}

#[cfg(test)]
mod distributed_builder_tests {
    use super::*;
    use crate::*;

    /// Test that `build_aqueduct` works for `LazyStreamRx<StreamSimpleMessage>` with `AqueTech::Aeron`.
    #[test]
    fn test_build_aqueduct_lazy_stream_rx() {
        let mut graph = GraphBuilder::for_testing().build(());
        if graph.aeron_media_driver().is_some() {
            let cb = graph.channel_builder();

            let (_lazy_tx, lazy_rx) = cb.build_stream(100);
            // Use AqueTech::Aeron with None for media_driver and dummy channel/stream_id
            let tech = AqueTech::None;
            // Create a minimal ActorBuilder and Threading
            let actor_builder = ActorBuilder::new(&mut graph).never_simulate(true);
            let threading = ScheduleAs::SoloAct;
            // Call build_aqueduct; test passes if it doesn't panic
            lazy_rx.build_aqueduct(tech, &actor_builder, threading);
        }
    }

    /// Test that `build_aqueduct` works for `LazyStreamTx<StreamSessionMessage>` with `AqueTech::Aeron`.
    #[test]
    fn test_build_aqueduct_lazy_stream_tx() {
        let mut graph = GraphBuilder::for_testing().build(());

        let cb = graph.channel_builder();

        let (lazy_tx, _lazy_rx) = cb.build_stream(100);
        let tech = AqueTech::None;
        let actor_builder = ActorBuilder::new(&mut graph).never_simulate(true);

        lazy_tx.build_aqueduct(tech, &actor_builder, SoloAct);
    }

    /// Test that `build_aqueduct` works for `LazySteadyStreamRxBundle<StreamSimpleMessage, GIRTH>` with `AqueTech::Aeron`.
    #[test]
    fn test_build_aqueduct_lazy_stream_rx_bundle_simple() {
        let mut graph = GraphBuilder::for_testing().build(());

        const GIRTH: usize = 1;
        let cb = graph.channel_builder();

        let (_lazy_tx, lazy_rx_bundle) = cb.build_stream_bundle::<StreamEgress, GIRTH>(100);
        let tech = AqueTech::None;
        let actor_builder = ActorBuilder::new(&mut graph).never_simulate(true);
        lazy_rx_bundle.build_aqueduct(tech, &actor_builder, SoloAct);
    }

    #[test]
    fn test_build_aqueduct_lazy_stream_tx_bundle_session() {
        let mut graph = GraphBuilder::for_testing().build(());

        const GIRTH: usize = 1;
        let cb = graph.channel_builder();

        let (lazy_tx_bundle, _lazy_rx_bundle) = cb.build_stream_bundle::<StreamIngress, GIRTH>(100);

        let tech = AqueTech::None;
        let actor_builder = ActorBuilder::new(&mut graph).never_simulate(true);
        lazy_tx_bundle.build_aqueduct(tech, &actor_builder, SoloAct);
    }
}