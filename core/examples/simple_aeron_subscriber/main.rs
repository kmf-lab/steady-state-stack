use std::env;
use std::time::Duration;
use steady_state::*;
use structopt::*;
use structopt_derive::*;

pub(crate) mod actor {
   pub(crate) mod subscriber;
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
    env::set_var("TELEMETRY_SERVER_PORT", "9102");
    env::set_var("TELEMETRY_SERVER_IP", "127.0.0.1");

    let cli_args = MainArg::from_args();
    let _ = init_logging("info");
    let mut graph = GraphBuilder::default()
           .with_telemtry_production_rate_ms(200)
           .build(cli_args); //or pass () if no args

    let aeron_channel: Channel = AeronConfig::new()
        .with_media_type(MediaType::Ipc) // 10MMps

        //   .with_media_type(MediaType::Udp)// 4MMps- std 4K page
        //   .with_term_length((1024 * 1024 * TERM_MB) as usize)

        .use_point_to_point(Endpoint {
            ip: "127.0.0.1".parse().expect("Invalid IP address"),
            port: 40456,
        })
        .build();

    if !graph.is_aeron_media_driver_present() {
        info!("aeron test skipped, no media driver present");
        return;
    }

    let channel_builder = graph.channel_builder();

//TODO: rename as string bundle?
    let (from_aeron_tx,from_aeron_rx) = channel_builder
        .with_avg_rate()
        .with_avg_filled()
        .with_filled_trigger(Trigger::AvgAbove(Filled::p50()), AlertColor::Yellow)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p70()), AlertColor::Orange)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
        .with_capacity(4*1024*1024)
        .build_as_stream_bundle::<StreamSessionMessage,1>(STREAM_ID
                                                          ,8);


    let base = from_aeron_rx.clone();
    //set this up first so sender has a place to send to
    graph.actor_builder().with_name("MockReceiver")
        .with_thread_info()
        .with_mcpu_percentile(Percentile::p96())
        .with_mcpu_percentile(Percentile::p25())
        .build(move |context| actor::subscriber::run(context, base.clone())
               , &mut Threading::Spawn);

    graph.build_stream_collector_bundle(AqueTech::Aeron(aeron_channel)
                                        , "ReceiverTest"
                                        , from_aeron_tx
                                        , &mut Threading::Spawn);

    graph.start(); //startup the graph
    graph.block_until_stopped(Duration::from_secs(21));
}
