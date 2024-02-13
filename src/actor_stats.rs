
use std::sync::Arc;

#[allow(unused_imports)]
use log::*;

use crate::{config,  Percentile, StdDev, Trigger};
use crate::monitor::{ActorMetaData};

#[derive(Default)]
pub struct ActorStatsComputer {

    pub(crate) red_trigger: Vec<Trigger>, //if used base is green
    pub(crate) yellow_trigger: Vec<Trigger>, //if used base is green

    pub(crate) refresh_rate_in_bits: u8,
    pub(crate) window_bucket_in_bits: u8,

    pub(crate) frame_rate_ms:u128, //const at runtime but needed here for unit testing
    pub(crate) time_label: String,
    pub(crate) percentiles_mcpu: Vec<Percentile>, //to show
    pub(crate) percentiles_work: Vec<Percentile>, //to show
    pub(crate) std_dev_mcpu: Vec<StdDev>, //to show
    pub(crate) std_dev_work: Vec<StdDev>, //to show
    pub(crate) show_avg_mcpu: bool,
    pub(crate) show_avg_work: bool,
    pub(crate) usage_review: bool,
}

impl ActorStatsComputer {


    pub(crate) fn init(&mut self, meta: Arc<ActorMetaData>) {

        self.red_trigger = meta.red.clone();
        self.yellow_trigger = meta.yellow.clone();

        self.frame_rate_ms = config::TELEMETRY_PRODUCTION_RATE_MS as u128;
        self.refresh_rate_in_bits = meta.refresh_rate_in_bits;
        self.window_bucket_in_bits = meta.window_bucket_in_bits;
        self.time_label = crate::channel_stats::time_label( self.frame_rate_ms << (meta.refresh_rate_in_bits+meta.window_bucket_in_bits) );

        self.show_avg_mcpu = meta.avg_mcpu;
        self.show_avg_work = meta.avg_work;
        self.percentiles_mcpu = meta.percentiles_mcpu.clone();
        self.percentiles_work = meta.percentiles_work.clone();
        self.std_dev_mcpu = meta.std_dev_mcpu.clone();
        self.std_dev_work  = meta.std_dev_work.clone();
        self.usage_review = meta.usage_review;

    }



    pub fn with_red(&mut self, trigger: Trigger) -> &mut Self {
        self.red_trigger.push(trigger);
        self
    }

    pub fn with_yellow(&mut self, trigger: Trigger) -> &mut Self {
        self.yellow_trigger.push(trigger);
        self
    }

}