use std::collections::VecDeque;
use std::sync::Arc;
use hdrhistogram::Histogram;

#[allow(unused_imports)]
use log::*;

use crate::{config,  Percentile, StdDev, Trigger};
use crate::channel_stats::ChannelBlock;
use crate::monitor::{ActorMetaData};

#[derive(Default)]
pub struct ActorStatsComputer {
    pub(crate) id: usize,
    pub(crate) name: &'static str,

    pub(crate) red_trigger: Vec<Trigger>, //if used base is green
    pub(crate) yellow_trigger: Vec<Trigger>, //if used base is green
    pub(crate) bucket_frames_count: usize, //when this bucket is full we add a new one

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

    pub(crate) history_mcpu: VecDeque<ChannelBlock>,
    pub(crate) history_work: VecDeque<ChannelBlock>,

    pub(crate) current_mcpu: Option<ChannelBlock>,
    pub(crate) current_work: Option<ChannelBlock>,

}

impl ActorStatsComputer {
    const PLACES_TENS:u64 = 1000u64;

    pub(crate) fn compute(&mut self
                          , id: usize
                          , mcpu: u64
                          , work: u64
                          , total_count_restarts: u32
                          , bool_stop: bool
                          , redundancy: u16) -> (String, &'static str) {

        self.accumulate_data_frame(mcpu, work);
        let mut display_label = String::new();

        if  0 != self.window_bucket_in_bits {
            display_label.push_str("per ");
            display_label.push_str(&self.time_label);
            display_label.push('\n');
        }

        // TODO:         self.append_computed_labels(&mut display_label);

        /*

                //  with_restarts
                //  with_instance_count  and is stopped
                println!("{} {} {}mCPU {}/{} {:.1}%Workload  restart:{} stop:{} redundancy:{}"
                          ,self.id
                         , self.display_label
                         , m_cpu, num, den, 100f32*workload
                         , actor_status.total_count_restarts
                         , actor_status.bool_stop
                         , actor_status.redundancy);// confirm
        */


        (self.name.to_string(),"blue")
    }


    pub(crate) fn init(&mut self, meta: Arc<ActorMetaData>
                       , id:usize, name:&'static str) {

        self.id = id;
        self.name = name;

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

        let mcpu_top = Self::PLACES_TENS * 1024 as u64;
        match Histogram::<u64>::new_with_bounds(1, mcpu_top, 2) {
            Ok(h) => {
                self.history_mcpu.push_back(ChannelBlock {
                    histogram: h, runner: 0, sum_of_squares: 0,
                });
            }
            Err(e) => {
                error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}"
                             ,mcpu_top, 2, e);
            }
        }

        let work_top = Self::PLACES_TENS * 100 as u64;
        match Histogram::<u64>::new_with_bounds(1, work_top, 2) {
            Ok(h) => {
                self.history_work.push_back(ChannelBlock {
                    histogram: h, runner: 0, sum_of_squares: 0,
                });
            }
            Err(e) => {
                error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}"
                             ,work_top, 2, e);
            }
        }

    }

    pub(crate) fn accumulate_data_frame(&mut self, mcpu: u64, work: u64) {

        let mcpu:u64 = Self::PLACES_TENS*mcpu;
        let work:u64 = Self::PLACES_TENS*work;

        self.history_mcpu.iter_mut().for_each(|f| {
            if let Err(e) = f.histogram.record(mcpu) {
                error!("unexpected, unable to record inflight {} err: {}", mcpu, e);
                panic!("oops");
            } else {
                f.runner = f.runner.saturating_add(mcpu as u128);
                f.sum_of_squares = f.sum_of_squares.saturating_add((mcpu as u128).pow(2));
            }
        });

        self.history_work.iter_mut().for_each(|f| {
            if let Err(e) = f.histogram.record(work) {
                error!("unexpected, unable to record inflight {} err: {}", work, e);
            } else {
                f.runner = f.runner.saturating_add(work as u128);
                f.sum_of_squares = f.sum_of_squares.saturating_add((work as u128).pow(2));
            }
        });


        self.bucket_frames_count += 1;
        if self.bucket_frames_count >= (1<<self.refresh_rate_in_bits) {
            self.bucket_frames_count = 0;

            if self.history_mcpu.len() >= (1<<self.window_bucket_in_bits) {
                self.current_mcpu = self.history_mcpu.pop_front();
            }
            if self.history_work.len() >= (1<<self.window_bucket_in_bits) {
                self.current_work = self.history_work.pop_front();
            }

            let mcpu_top = Self::PLACES_TENS * 1024 as u64;
            match Histogram::<u64>::new_with_bounds(1, mcpu_top, 2) {
                Ok(h) => {
                    self.history_mcpu.push_back(ChannelBlock {
                        histogram: h, runner: 0, sum_of_squares: 0,
                    });
                }
                Err(e) => {
                    error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}"
                             ,mcpu_top, 2, e);
                }
            }

            let work_top = Self::PLACES_TENS * 100 as u64;
            match Histogram::<u64>::new_with_bounds(1, work_top, 2) {
                Ok(h) => {
                    self.history_work.push_back(ChannelBlock {
                        histogram: h, runner: 0, sum_of_squares: 0,
                    });
                }
                Err(e) => {
                    error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}"
                             ,work_top, 2, e);
                }
            }


        }
    }



}