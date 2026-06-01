use std::env;
use std::time::Duration;
use steady_state::*;
use steady_state::distributed::aqueduct_builder::AqueductBuilder;


pub(crate) mod actor {
   pub(crate) mod subscriber;
}

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct MainArg {
    #[arg(short = 'r', long = "rate", default_value = "1000")]
    pub(crate) rate_ms: u64,
    #[arg(short = 'b', long = "beats", default_value = "60")]
    pub(crate) beats: u64,
}

/// test ID for the unit tests
pub const STREAM_ID: i32 = 1234;

//  https://github.com/real-logic/aeron/wiki/Best-Practices-Guide


fn main() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        env::set_var("TELEMETRY_SERVER_PORT",
        "9102");
        env::set_var("TELEMETRY_SERVER_IP",
        "127.0.0.1");
    }

    let cli_args = MainArg::parse();
    let _ = init_logging(LogLevel::Info, None);
    let mut graph = GraphBuilder::default()
           .with_telemtry_production_rate_ms(200)
           .build(cli_args); //or pass () if no args

    // Must match publisher: IPC. See core/tests/common/support/pub_sub_harness.rs.
    let aeron_channel = AeronConfig::new()
        .with_media_type(MediaType::Ipc)
        .use_ipc()
        .build();

    if graph.aeron_media_driver().is_none() {
        info!("aeron test skipped, no media driver present");
        return Ok(());
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
        .build_stream_bundle::<StreamIngress,1>(8);


    let base = from_aeron_rx.clone();
    //set this up first so sender has a place to send to
    graph.actor_builder().with_name("MockReceiver")
        .with_thread_info()
        .with_mcpu_percentile(Percentile::p96())
        .with_mcpu_percentile(Percentile::p25())
        .build(move |context| actor::subscriber::run(context, base.clone())
               , ScheduleAs::SoloAct);

    from_aeron_tx.build_aqueduct(AqueTech::Aeron(aeron_channel, STREAM_ID)
                                 , &graph.actor_builder().with_name("ReceiverTest").never_simulate(false)
                                 , ScheduleAs::SoloAct);

    graph.start(); //startup the graph
    graph.block_until_stopped(Duration::from_secs(21))
}
