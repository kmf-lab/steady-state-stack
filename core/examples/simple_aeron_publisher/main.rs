use std::env;
use std::time::Duration;
use log::info;
use steady_state::*;
use structopt::*;
use steady_state::distributed::distributed_builder::AqueductBuilder;

pub(crate) mod actor {
   pub(crate) mod publisher;
}

#[derive(StructOpt, Debug, PartialEq, Clone)]
pub(crate) struct MainArg {
    #[structopt(short = "r", long = "rate", default_value = "1000")]
    pub(crate) rate_ms: u64,
    #[structopt(short = "b", long = "beats", default_value = "60")]
    pub(crate) beats: u64,
}

pub const STREAM_ID: i32 = 1234;
//  https://github.com/real-logic/aeron/wiki/Best-Practices-Guide


fn main() {

    env::set_var("TELEMETRY_SERVER_PORT", "9101");
    env::set_var("TELEMETRY_SERVER_IP", "127.0.0.1");

    let cli_args = MainArg::from_args();
    let _ = init_logging("info");
    let mut graph = GraphBuilder::default()
           .with_telemtry_production_rate_ms(200)
           .build(cli_args); //or pass () if no args

    let aeron = graph.aeron_md();
    if aeron.is_none() {
        info!("aeron test skipped, no media driver present");
        return;
    }
    let aeron_channel = AeronConfig::new()
            .with_media_type(MediaType::Ipc)
            .use_ipc()
            .build();



    let channel_builder = graph.channel_builder();

    let (to_aeron_tx, to_aeron_rx) = channel_builder
        .with_avg_rate()
        .with_avg_filled()
        .with_filled_trigger(Trigger::AvgAbove(Filled::p50()), AlertColor::Yellow)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p70()), AlertColor::Orange)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
        .with_capacity(4*1024*1024)
        .build_as_stream_bundle::<StreamSimpleMessage,1>(STREAM_ID
                                                         ,8);




    graph.actor_builder().with_name("MockSender")
        .with_thread_info()
        .with_mcpu_percentile(Percentile::p96())
        .with_mcpu_percentile(Percentile::p25())
        .build(move |context| actor::publisher::run(context, to_aeron_tx.clone())
               , &mut Threading::Spawn);


    to_aeron_rx.build_aqueduct( AqueTech::Aeron(graph.aeron_md(), aeron_channel)
                             , &graph.actor_builder().with_name("SenderTest")
                             , &mut Threading::Spawn);




    graph.start(); //startup the graph
    graph.block_until_stopped(Duration::from_secs(21));
}
