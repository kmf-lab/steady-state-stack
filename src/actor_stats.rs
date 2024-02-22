use std::collections::VecDeque;
use std::sync::Arc;
use hdrhistogram::Histogram;

#[allow(unused_imports)]
use log::*;

use crate::*;
use crate::channel_stats::{ChannelBlock, compute_labels, DOT_GREEN, DOT_GREY, DOT_ORANGE, DOT_RED, DOT_YELLOW};
use crate::monitor::{ActorMetaData};

#[derive(Default)]
pub struct ActorStatsComputer {
    pub(crate) id: usize,
    pub(crate) name: &'static str,

    pub(crate) mcpu_trigger: Vec<(Trigger<MCPU>,AlertColor)>, //if used base is green
    pub(crate) work_trigger: Vec<(Trigger<Work>,AlertColor)>, //if used base is green
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

    pub(crate) build_mcpu_histogram: bool,
    pub(crate) build_work_histogram: bool,

    pub(crate) history_mcpu: VecDeque<ChannelBlock<u16>>,
    pub(crate) history_work: VecDeque<ChannelBlock<u8>>,

    pub(crate) current_mcpu: Option<ChannelBlock<u16>>,
    pub(crate) current_work: Option<ChannelBlock<u8>>,

}

impl ActorStatsComputer {
    const PLACES_TENS:u64 = 1000u64;

    pub(crate) fn compute(&mut self
                          , mcpu: u64
                          , work: u64
                          , total_count_restarts: u32
                          , bool_stop: bool
                          , redundancy: u16) -> (String, &'static str, &'static str) {

        self.accumulate_data_frame(mcpu, work);
        let mut display_label = String::new();

        display_label.push('#');
        display_label.push_str(itoa::Buffer::new().format(self.id));
        display_label.push(' ');
        display_label.push_str(self.name);
        display_label.push('\n');


        if  0 != self.window_bucket_in_bits {
            display_label.push_str("Window ");
            display_label.push_str(&self.time_label);
            display_label.push('\n');
        }

        if total_count_restarts>0 {
            display_label.push_str("restarts: ");
            display_label.push_str(itoa::Buffer::new().format(total_count_restarts));
            display_label.push('\n');
        }
        if redundancy>1 {
            display_label.push_str("redundancy: ");
            display_label.push_str(itoa::Buffer::new().format(redundancy));
            display_label.push('\n');
        }
        if bool_stop {
            display_label.push_str("stopped");
            display_label.push('\n');
        }



        if let Some(ref current_work) = &self.current_work {
            compute_labels(self.frame_rate_ms
                           , self.window_bucket_in_bits+self.refresh_rate_in_bits
                           , &mut display_label, current_work
                           , "work"
                           , "%"
                           , (1,1)
                           , self.show_avg_work
                           , & self.std_dev_work
                           , & self.percentiles_work);
        }
        if let Some(ref current_mcpu) = &self.current_mcpu {
            compute_labels(self.frame_rate_ms
                           , self.window_bucket_in_bits+self.refresh_rate_in_bits
                           , &mut display_label, current_mcpu
                           , "mCPU"
                           , ""
                           , (1,1)
                           , self.show_avg_mcpu
                           , & self.std_dev_mcpu
                           , & self.percentiles_mcpu);
        }


        let mut line_color = DOT_GREY;
        if !self.mcpu_trigger.is_empty() || !self.work_trigger.is_empty() {
            line_color = DOT_GREEN;
            if self.trigger_alert_level(&AlertColor::Yellow) {
                line_color = DOT_YELLOW;
            };
            if self.trigger_alert_level(&AlertColor::Orange) {
                line_color = DOT_ORANGE;
            };
            if self.trigger_alert_level(&AlertColor::Red) {
                line_color = DOT_RED;
            };
        }
        let line_width = "2";


        (display_label,line_color,line_width)
    }


    pub(crate) fn init(&mut self, meta: Arc<ActorMetaData>
                       , id:usize
                       , name:&'static str) {

        self.id = id;
        self.name = name;

        //TODO: we could pre-filter these by color here since they will not change again
        //      this might be needed for faster updates..
        self.mcpu_trigger = meta.trigger_mcpu.clone();
        self.work_trigger = meta.trigger_work.clone();

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

        let trigger_uses_histogram = self.mcpu_trigger.iter().any(|t|
            matches!(t, (Trigger::PercentileAbove(_,_),_) | (Trigger::PercentileBelow(_,_),_))
        );

        self.build_mcpu_histogram = trigger_uses_histogram || !self.percentiles_mcpu.is_empty();

        if self.build_mcpu_histogram {
            let mcpu_top = 1024;
            match Histogram::<u16>::new_with_bounds(1, mcpu_top, 1) {
                Ok(h) => {
                    self.history_mcpu.push_back(ChannelBlock {
                        histogram: Some(h),
                        runner: 0,
                        sum_of_squares: 0,
                    });
                }
                Err(e) => {
                    error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}"
                             ,mcpu_top, 2, e);
                }
            }
        } else {
            self.history_mcpu.push_back(ChannelBlock {
                histogram: None,
                runner: 0,
                sum_of_squares: 0,
            });
        }

        let trigger_uses_histogram = self.work_trigger.iter().any( |t|
            matches!(t, (Trigger::PercentileAbove(_,_),_) | (Trigger::PercentileBelow(_,_),_))
        );


        self.build_work_histogram = trigger_uses_histogram || !self.percentiles_work.is_empty();

        if self.build_work_histogram {
            let work_top = 100;
            match Histogram::<u8>::new_with_bounds(1, work_top, 1) {
                Ok(h) => {
                    self.history_work.push_back(ChannelBlock {
                        histogram: Some(h),
                        runner: 0,
                        sum_of_squares: 0,
                    });
                }
                Err(e) => {
                    error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}"
                             ,work_top, 2, e);
                }
            }
        } else {
            self.history_work.push_back(ChannelBlock {
                histogram: None,
                runner: 0,
                sum_of_squares: 0,
            });
        }

    }

    pub(crate) fn accumulate_data_frame(&mut self, mcpu: u64, work: u64) {

        self.history_mcpu.iter_mut().for_each(|f| {
            if let Some(ref mut h) = &mut f.histogram {
                if let Err(e) = h.record(mcpu) {
                    error!("unexpected, unable to record inflight {} err: {}", mcpu, e);
                }
            }
            let mcpu = mcpu * Self::PLACES_TENS;
            f.runner = f.runner.saturating_add(mcpu as u128);
            f.sum_of_squares = f.sum_of_squares.saturating_add((mcpu as u128).pow(2));

        });

        self.history_work.iter_mut().for_each(|f| {
            if let Some(ref mut h) = &mut f.histogram {
                if let Err(e) = h.record(work) {
                    error!("unexpected, unable to record inflight {} err: {}", work, e);
                }
            }
            let work = work * Self::PLACES_TENS;
            f.runner = f.runner.saturating_add(work as u128);
            f.sum_of_squares = f.sum_of_squares.saturating_add((work as u128).pow(2));

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

            if self.build_mcpu_histogram {
                let mcpu_top = 1024;
                match Histogram::<u16>::new_with_bounds(1, mcpu_top, 1) {
                    Ok(h) => {
                        self.history_mcpu.push_back(ChannelBlock {
                            histogram: Some(h),
                            runner: 0,
                            sum_of_squares: 0,
                        });
                    }
                    Err(e) => {
                        error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}"
                             ,mcpu_top, 2, e);
                    }
                }
            } else {
                self.history_mcpu.push_back(ChannelBlock {
                    histogram: None,
                    runner: 0,
                    sum_of_squares: 0,
                });
            }

            if self.build_work_histogram {
                let work_top = 100;
                match Histogram::<u8>::new_with_bounds(1, work_top, 1) {
                    Ok(h) => {
                        self.history_work.push_back(ChannelBlock {
                            histogram: Some(h),
                            runner: 0,
                            sum_of_squares: 0,
                        });
                    }
                    Err(e) => {
                        error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}"
                             ,work_top, 2, e);
                    }
                }
            } else {
                self.history_work.push_back(ChannelBlock {
                    histogram: None,
                    runner: 0,
                    sum_of_squares: 0,
                });
            }


        }
    }


    fn trigger_alert_level(&mut self, c1: &AlertColor) -> bool {
        self.mcpu_trigger.iter().filter(|f| f.1.eq(c1)).any(|f| self.triggered_mcpu(&f.0))
            || self.work_trigger.iter().filter(|f| f.1.eq(c1)).any(|f| self.triggered_work(&f.0))
    }


    fn triggered_mcpu(&self, rule: &Trigger<MCPU>) -> bool
    {
        match rule {
            Trigger::AvgBelow(mcpu) => {
                let window_in_ms = self.frame_rate_ms << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
                channel_stats::avg_rational(window_in_ms, &self.current_mcpu, mcpu.rational()).is_lt()
            },
            Trigger::AvgAbove(mcpu) => {
                let window_in_ms = self.frame_rate_ms << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
                channel_stats::avg_rational(window_in_ms, &self.current_mcpu, mcpu.rational()).is_gt()
            },
            Trigger::StdDevsBelow(std_devs, mcpu) => {
                let window_bits = self.window_bucket_in_bits + self.refresh_rate_in_bits;
                channel_stats::stddev_rational(self.mcpu_std_dev(), window_bits, std_devs, &self.current_mcpu, mcpu.rational()).is_lt()
            }
            Trigger::StdDevsAbove(std_devs, mcpu) => {
                let window_bits = self.window_bucket_in_bits + self.refresh_rate_in_bits;
                channel_stats::stddev_rational(self.mcpu_std_dev(), window_bits, std_devs, &self.current_mcpu, mcpu.rational()).is_gt()
            }
            Trigger::PercentileAbove(percentile, mcpu) => {
                channel_stats::percentile_rational(percentile, &self.current_mcpu, mcpu.rational()).is_gt()
            }
            Trigger::PercentileBelow(percentile, mcpu) => {
                channel_stats::percentile_rational(percentile, &self.current_mcpu, mcpu.rational()).is_lt()
            }

        }
    }

    fn triggered_work(&self, rule: &Trigger<Work>) -> bool
    {
        match rule {
            Trigger::AvgBelow(work) => {
                let window_in_ms = self.frame_rate_ms << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
                channel_stats::avg_rational(window_in_ms,&self.current_work, work.rational()).is_lt()
            },
            Trigger::AvgAbove(work) => {
                let window_in_ms = self.frame_rate_ms << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
                channel_stats::avg_rational(window_in_ms,&self.current_work, work.rational()).is_gt()
            },
            Trigger::StdDevsBelow(std_devs, work) => {
                let window_bits = self.window_bucket_in_bits + self.refresh_rate_in_bits;
                channel_stats::stddev_rational( self.work_std_dev(), window_bits, std_devs, &self.current_work, work.rational()).is_lt()
            }
            Trigger::StdDevsAbove(std_devs, work) => {
                let window_bits = self.window_bucket_in_bits + self.refresh_rate_in_bits;
                channel_stats::stddev_rational(self.work_std_dev(),  window_bits, std_devs, &self.current_work, work.rational()).is_gt()
            }
            Trigger::PercentileAbove(percentile, work) => {
                channel_stats::percentile_rational(percentile, &self.current_work, work.rational()).is_gt()
            }
            Trigger::PercentileBelow(percentile, work) => {
                channel_stats::percentile_rational(percentile, &self.current_work, work.rational()).is_lt()
            }

        }
    }

    #[inline]
    fn mcpu_std_dev(&self) -> f32 {
        if let Some(c) = &self.current_mcpu {
            channel_stats::compute_std_dev(self.refresh_rate_in_bits + self.window_bucket_in_bits
                                  , 1 << (self.refresh_rate_in_bits + self.window_bucket_in_bits)
                                  , c.runner
                                  , c.sum_of_squares)
        } else {
            info!("skipping std no current data");
            0f32
        }
    }
    #[inline]
    fn work_std_dev(&self) -> f32 {
        if let Some(c) = &self.current_work {
            channel_stats::compute_std_dev(self.refresh_rate_in_bits + self.window_bucket_in_bits
                                  , 1 << (self.refresh_rate_in_bits + self.window_bucket_in_bits)
                                  , c.runner
                                  , c.sum_of_squares)
        } else {
            info!("skipping std no current data");
            0f32
        }
    }


}
