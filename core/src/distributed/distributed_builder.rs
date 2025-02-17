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
        threading: &mut Threading,
    );
}

impl AqueductBuilder for LazyStreamRx<StreamSimpleMessage> {
    fn build_aqueduct(
        self,
        tech: AqueTech,
        actor_builder: &ActorBuilder,
        threading: &mut Threading,
    ) {
        match tech {
            AqueTech::Aeron(media_driver, channel) => {
                let actor_builder = actor_builder.clone();
                if let Some(aeron) = media_driver {
                    let state = new_state();
                    actor_builder.build(move |context|
                                   aeron_publish::run(context
                                                      , self.clone()
                                                      , channel.clone()
                                                      , aeron.clone()
                                                      , state.clone())
                               , threading)
                }
            }
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
        threading: &mut Threading,
    ) {
        match tech {
            AqueTech::Aeron(media_driver, channel) => {
                let actor_builder = actor_builder.clone();
                if let Some(aeron) = media_driver {
                    let state = new_state();
                    actor_builder.build(move |context|
                                   aeron_subscribe::run(context
                                                        , self.clone() //tx: SteadyStreamTxBundle<StreamFragment,GIRTH>
                                                        , channel.clone()
                                                        , aeron.clone()
                                                        , state.clone())
                               , threading);
                }
            }
            _ => {
                panic!("unsupported distribution type");
            }
        }
    }
}
impl<const GIRTH: usize> AqueductBuilder for LazySteadyStreamRxBundle<StreamSimpleMessage, GIRTH> {
    fn build_aqueduct(
        self,
        tech: AqueTech,
        actor_builder: &ActorBuilder,
        threading: &mut Threading,
    ) {
        match tech {
            AqueTech::Aeron(media_driver, channel) => {
                let actor_builder = actor_builder.clone();
                if let Some(aeron) = media_driver {
                    let state = new_state();
                    actor_builder.build(move |context|
                                   aeron_publish_bundle::run(context
                                                             , self.clone()
                                                             , channel.clone()
                                                             , aeron.clone()
                                                             , state.clone())
                               , threading)
                }
            }
            _ => {
                panic!("unsupported distribution type");
            }
        }
    }
}

impl<const GIRTH: usize> AqueductBuilder for LazySteadyStreamTxBundle<StreamSessionMessage, GIRTH> {
    fn build_aqueduct(
        self,
        tech: AqueTech,
        actor_builder: &ActorBuilder,
        threading: &mut Threading,
    ) {
        match tech {
            AqueTech::Aeron(media_driver, channel) => {
                let actor_builder = actor_builder.clone();
                if let Some(aeron) = media_driver {
                    let state = new_state();
                    actor_builder.build(move |context|
                                   aeron_subscribe_bundle::run(context
                                                               , self.clone() //tx: SteadyStreamTxBundle<StreamFragment,GIRTH>
                                                               , channel.clone()
                                                               , aeron.clone()
                                                               , state.clone())
                               , threading);
                }
            }
            _ => {
                panic!("unsupported distribution type");
            }
        }
    }
}
