use aeron::context::Context;
use crate::{new_state, Graph, LazyStreamRx, LazyStreamTx, Percentile, Threading};
use crate::distributed::aeron_channel_structs::aeron_utils::aeron_context;
use crate::distributed::aeron_channel_builder::AqueTech;
use crate::distributed::{aeron_publish, aeron_publish_bundle, aeron_subscribe, aeron_subscribe_bundle, distributed_builder};
use crate::distributed::distributed_stream::{LazySteadyStreamRxBundle, LazySteadyStreamRxBundleClone, LazySteadyStreamTxBundle, LazySteadyStreamTxBundleClone, StreamSessionMessage, StreamSimpleMessage};

pub trait AqueductBuilder {
    fn build_aqueduct(
        self,
        graph: &mut Graph,
        tech: AqueTech,
        name: &'static str,
        threading: &mut Threading,
    );
}

impl AqueductBuilder for LazyStreamRx<StreamSimpleMessage> {
    fn build_aqueduct(
        self,
        graph: &mut Graph,
        tech: AqueTech,
        name: &'static str,
        threading: &mut Threading,
    ) {
        build_distributor_single(graph, tech, name, self, threading);
    }
}

impl AqueductBuilder for LazyStreamTx<StreamSessionMessage> {
    fn build_aqueduct(
        self,
        graph: &mut Graph,
        tech: AqueTech,
        name: &'static str,
        threading: &mut Threading,
    ) {
        build_collector_single(graph, tech, name, self, threading);
    }
}
impl<const GIRTH: usize> AqueductBuilder for LazySteadyStreamRxBundle<StreamSimpleMessage, GIRTH> {
    fn build_aqueduct(
        self,
        graph: &mut Graph,
        tech: AqueTech,
        name: &'static str,
        threading: &mut Threading,
    ) {
        build_distributor_bundle(graph, tech, name, self, threading);
    }
}

impl<const GIRTH: usize> AqueductBuilder for LazySteadyStreamTxBundle<StreamSessionMessage, GIRTH> {
    fn build_aqueduct(
        self,
        graph: &mut Graph,
        tech: AqueTech,
        name: &'static str,
        threading: &mut Threading,
    ) {
        build_collector_bundle(graph, tech, name, self, threading);
    }
}


pub(crate) fn build_collector_single(g: &mut Graph, distribution: AqueTech, name: &'static str, tx: LazyStreamTx<StreamSessionMessage>, threading: &mut Threading) {
    match distribution {
        AqueTech::Aeron(channel) => {
            if g.aeron.is_none() { //lazy load, we only support one
                g.aeron = aeron_context(Context::new());
            }

            if let Some(ref aeron) = &g.aeron {
                let state = new_state();
                let aeron = aeron.clone();

                //let connection = channel.cstring();

                g.actor_builder()
                    .with_name(name)
                    .with_thread_info()
                    .with_mcpu_percentile(Percentile::p96())
                    .with_mcpu_percentile(Percentile::p25())

                    //  .with_explicit_core(8)
                    //.with_custom_label(connection) // TODO: need something like this.
                    .build(move |context|
                               aeron_subscribe::run(context
                                                    , tx.clone() //tx: SteadyStreamTxBundle<StreamFragment,GIRTH>
                                                    , channel
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



pub(crate) fn build_collector_bundle<const GIRTH:usize>(g: &mut Graph, distribution: AqueTech, name: &'static str, tx: LazySteadyStreamTxBundle<StreamSessionMessage, { GIRTH }>, threading: &mut Threading) {
    match distribution {
        AqueTech::Aeron(channel) => {
            if g.aeron.is_none() { //lazy load, we only support one
                g.aeron = aeron_context(Context::new());
            }

            if let Some(ref aeron) = &g.aeron {
                let state = new_state();
                let aeron = aeron.clone();

                //let connection = channel.cstring();

                g.actor_builder()
                    .with_name(name)
                    .with_thread_info()
                    .with_mcpu_percentile(Percentile::p96())
                    .with_mcpu_percentile(Percentile::p25())

                    //  .with_explicit_core(8)
                    //.with_custom_label(connection) // TODO: need something like this.
                    .build(move |context|
                               aeron_subscribe_bundle::run(context
                                                           , tx.clone() //tx: SteadyStreamTxBundle<StreamFragment,GIRTH>
                                                           , channel
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

pub(crate) fn build_distributor_bundle<const GIRTH:usize>(g: &mut Graph, distribution: AqueTech, name: &'static str, rx: LazySteadyStreamRxBundle<StreamSimpleMessage, { GIRTH }>, threading: &mut Threading) {
    match distribution {
        AqueTech::Aeron(channel) => {
            if g.aeron.is_none() { //lazy load, we only support one
                g.aeron = aeron_context(Context::new());
            }

            if let Some(aeron) = &g.aeron {
                let state = new_state();
                let aeron = aeron.clone();

                g.actor_builder()
                    .with_name(name)
                    .with_thread_info()
                    .with_mcpu_percentile(Percentile::p96())
                    .with_mcpu_percentile(Percentile::p25())
                    // .with_explicit_core(7)
                    .build(move |context|
                               aeron_publish_bundle::run(context
                                                         , rx.clone()
                                                         , channel
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


pub(crate) fn build_distributor_single(g: &mut Graph, distribution: AqueTech, name: &'static str, rx: LazyStreamRx<StreamSimpleMessage>, threading: &mut Threading) {
    match distribution {
        AqueTech::Aeron(channel) => {
            if g.aeron.is_none() { //lazy load, we only support one
                g.aeron = aeron_context(Context::new());
            }

            if let Some(aeron) = &g.aeron {
                let state = new_state();
                let aeron = aeron.clone();

                g.actor_builder()
                    .with_name(name)
                    .with_thread_info()
                    .with_mcpu_percentile(Percentile::p96())
                    .with_mcpu_percentile(Percentile::p25())
                    // .with_explicit_core(7)
                    .build(move |context|
                               aeron_publish::run(context
                                                  , rx.clone()
                                                  , channel
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
