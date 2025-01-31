use std::time::Duration;
use log::info;
use steady_state::*;
use structopt::*;
use structopt_derive::*;

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
pub const AERON_CHANNEL: Channel = AeronConfig::new()
    .with_media_type(MediaType::Ipc) // 10MMps

    //   .with_media_type(MediaType::Udp)// 4MMps- std 4K page
    //   .with_term_length((1024 * 1024 * TERM_MB) as usize)

    .use_point_to_point(Endpoint {
        ip: "127.0.0.1".parse().expect("Invalid IP address"),
        port: 40456,
    })
    .build();

fn main() {
    let cli_args = MainArg::from_args();
    let _ = init_logging("info");
    let mut graph = GraphBuilder::default()
           .build(cli_args); //or pass () if no args

    let mut graph = GraphBuilder::for_testing()
        .with_telemetry_metric_features(true)
        .build(());

    if !graph.is_aeron_media_driver_present() {
        info!("aeron test skipped, no media driver present");
        return;
    }

    let channel_builder = graph.channel_builder();

    let (to_aeron_tx,to_aeron_rx) = channel_builder
        .with_avg_rate()
        .with_avg_filled()
        .with_filled_trigger(Trigger::AvgAbove(Filled::p50()), AlertColor::Yellow)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p70()), AlertColor::Orange)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
        .build_as_stream::<StreamSimpleMessage,1>(STREAM_ID
                                                  , 4*1024*1024
                                                  , 32*1024*1024);




    graph.actor_builder().with_name("MockSender")
        .with_thread_info()
        .with_mcpu_percentile(Percentile::p96())
        .with_mcpu_percentile(Percentile::p25())
        .build(move |context| actor::publisher::run(context, to_aeron_tx.clone())
               , &mut Threading::Spawn);

    graph.build_stream_distributor(DistributedTech::Aeron(AERON_CHANNEL)
                                   , "SenderTest"
                                   , to_aeron_rx
                                   , &mut Threading::Spawn);


    graph.start(); //startup the graph
    graph.block_until_stopped(Duration::from_secs(21));
}
