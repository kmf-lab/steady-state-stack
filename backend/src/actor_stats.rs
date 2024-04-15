use std::collections::VecDeque;

use hdrhistogram::{Counter, Histogram};

#[allow(unused_imports)]
use log::*;
use std::cmp;

use crate::*;
use crate::channel_stats::{compute_labels, DOT_GREEN, DOT_GREY, DOT_ORANGE, DOT_RED, DOT_YELLOW, PLACES_TENS};

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
                          , bool_stop: bool) -> (String, &'static str, &'static str) {

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

        if bool_stop {
            display_label.push_str("stopped");
            display_label.push('\n');
        }

        if let Some(ref current_work) = &self.current_work {
            compute_labels(self.frame_rate_ms
                           , self.window_bucket_in_bits+self.refresh_rate_in_bits
                           , &mut display_label, current_work
                           , "work", "%"
                           , 100
                           , (1,1)
                           , self.show_avg_work
                           , & self.std_dev_work
                           , & self.percentiles_work);
        }
        if let Some(ref current_mcpu) = &self.current_mcpu {
            compute_labels(self.frame_rate_ms
                           , self.window_bucket_in_bits+self.refresh_rate_in_bits
                           , &mut display_label, current_mcpu
                           , "mCPU", ""
                           , 1024
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

        //TODO: Perf, we could pre-filter these by color here since they will not change again
        //      this might be needed for faster updates..
        self.mcpu_trigger.clone_from(&meta.trigger_mcpu);
        self.work_trigger.clone_from(&meta.trigger_work);

        self.frame_rate_ms = config::TELEMETRY_PRODUCTION_RATE_MS as u128;
        self.refresh_rate_in_bits = meta.refresh_rate_in_bits;
        self.window_bucket_in_bits = meta.window_bucket_in_bits;
        self.time_label = time_label( self.frame_rate_ms << (meta.refresh_rate_in_bits+meta.window_bucket_in_bits) );

        self.show_avg_mcpu = meta.avg_mcpu;
        self.show_avg_work = meta.avg_work;
        self.percentiles_mcpu.clone_from(&meta.percentiles_mcpu);
        self.percentiles_work.clone_from(&meta.percentiles_work);
        self.std_dev_mcpu.clone_from(&meta.std_dev_mcpu);
        self.std_dev_work.clone_from(&meta.std_dev_work);
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
            self.history_mcpu.push_back(ChannelBlock::default());
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
            self.history_work.push_back(ChannelBlock::default());
        }

    }

    pub(crate) fn accumulate_data_frame(&mut self, mcpu: u64, work: u64) {

        assert!(mcpu<=1024,"mcpu out of range {}",mcpu);

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
                self.history_mcpu.push_back(ChannelBlock::default());
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
                self.history_work.push_back(ChannelBlock::default());
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
                avg_rational(window_in_ms, &self.current_mcpu, mcpu.rational()).is_lt()
            },
            Trigger::AvgAbove(mcpu) => {
                let window_in_ms = self.frame_rate_ms << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
                avg_rational(window_in_ms, &self.current_mcpu, mcpu.rational()).is_gt()
            },
            Trigger::StdDevsBelow(std_devs, mcpu) => {
                let window_bits = self.window_bucket_in_bits + self.refresh_rate_in_bits;
                stddev_rational(self.mcpu_std_dev(), window_bits, std_devs, &self.current_mcpu, mcpu.rational()).is_lt()
            }
            Trigger::StdDevsAbove(std_devs, mcpu) => {
                let window_bits = self.window_bucket_in_bits + self.refresh_rate_in_bits;
                stddev_rational(self.mcpu_std_dev(), window_bits, std_devs, &self.current_mcpu, mcpu.rational()).is_gt()
            }
            Trigger::PercentileAbove(percentile, mcpu) => {
                percentile_rational(percentile, &self.current_mcpu, mcpu.rational()).is_gt()
            }
            Trigger::PercentileBelow(percentile, mcpu) => {
                percentile_rational(percentile, &self.current_mcpu, mcpu.rational()).is_lt()
            }

        }
    }

    fn triggered_work(&self, rule: &Trigger<Work>) -> bool
    {
        match rule {
            Trigger::AvgBelow(work) => {
                let window_in_ms = self.frame_rate_ms << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
                avg_rational(window_in_ms, &self.current_work, work.rational()).is_lt()
            },
            Trigger::AvgAbove(work) => {
                let window_in_ms = self.frame_rate_ms << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
                avg_rational(window_in_ms, &self.current_work, work.rational()).is_gt()
            },
            Trigger::StdDevsBelow(std_devs, work) => {
                let window_bits = self.window_bucket_in_bits + self.refresh_rate_in_bits;
                stddev_rational(self.work_std_dev(), window_bits, std_devs, &self.current_work, work.rational()).is_lt()
            }
            Trigger::StdDevsAbove(std_devs, work) => {
                let window_bits = self.window_bucket_in_bits + self.refresh_rate_in_bits;
                stddev_rational(self.work_std_dev(), window_bits, std_devs, &self.current_work, work.rational()).is_gt()
            }
            Trigger::PercentileAbove(percentile, work) => {
                percentile_rational(percentile, &self.current_work, work.rational()).is_gt()
            }
            Trigger::PercentileBelow(percentile, work) => {
                percentile_rational(percentile, &self.current_work, work.rational()).is_lt()
            }

        }
    }

    #[inline]
    fn mcpu_std_dev(&self) -> f32 {
        if let Some(c) = &self.current_mcpu {
            compute_std_dev(self.refresh_rate_in_bits + self.window_bucket_in_bits
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
            compute_std_dev(self.refresh_rate_in_bits + self.window_bucket_in_bits
                            , 1 << (self.refresh_rate_in_bits + self.window_bucket_in_bits)
                            , c.runner
                            , c.sum_of_squares)
        } else {
            info!("skipping std no current data");
            0f32
        }
    }


}

pub(crate) fn time_label(total_ms: u128) -> String {
    let seconds = total_ms as f64 / 1000.0;
    let minutes = seconds / 60.0;
    let hours = minutes / 60.0;
    let days = hours / 24.0;

    if days >= 1.0 {
        if days< 1.1 {"day".to_string()} else {
            format!("{:.1} days", days)
        }
    } else if hours >= 1.0 {
        if hours< 1.1 {"hr".to_string()} else {
            format!("{:.1} hrs", hours)
        }
    } else if minutes >= 1.0 {
        if minutes< 1.1 {"min".to_string()} else {
            format!("{:.1} mins", minutes)
        }
    } else if seconds< 1.1 {"sec".to_string()} else {
        format!("{:.1} secs", seconds)
    }

}

//used for avg_rate, avg_mcpu and avg_work as a common implementation
pub(crate) fn avg_rational<T: Counter>(window_in_ms: u128, current: &Option<ChannelBlock<T>>, rational: (u64, u64)) -> cmp::Ordering {
    if let Some(current) = current {
        (current.runner * rational.1 as u128)
            .cmp(&(PLACES_TENS as u128 * window_in_ms * rational.0 as u128))
    } else {
        cmp::Ordering::Equal //unknown
    }
}

// self.inflight_std_dev()  (self.window_bucket_in_bits + self.refresh_rate_in_bits)
//used for avg_rate, avg_mcpu and avg_work as a common implementation
pub(crate) fn stddev_rational<T: Counter>(std_dev: f32, window_bits: u8
                                          , std_devs: &StdDev
                                          , current: &Option<ChannelBlock<T>>,
                                          expected: (u64,u64)  ) -> cmp::Ordering {
    if let Some(current) = current {

        let std_deviation = (std_dev * std_devs.value()) as u128;
       // let measured_value = ((current.runner >> window_bits) + std_deviation)/ PLACES_TENS as u128;
        //let expected_value = expected.0 as f32 * expected.1 as f32;
        //info!("stddev value: {} vs {} ", measured_value,expected_value);
        (expected.1 as u128 * ((current.runner >> window_bits) + std_deviation)).cmp( &(PLACES_TENS as u128 * expected.0 as u128) )

    } else {
        cmp::Ordering::Equal //unknown
    }
}

//used for avg_rate, avg_mcpu and avg_work as a common implementation
pub(crate) fn percentile_rational<T: Counter>(percentile: &Percentile, consumed: &Option<ChannelBlock<T>>, rational: (u64, u64)) -> cmp::Ordering {
    if let Some(current_consumed) = consumed {
        if let Some(h) = &current_consumed.histogram {
            let measured_rate_ms = h.value_at_percentile(percentile.percentile()) as u128;
            (measured_rate_ms * rational.1 as u128).cmp(&(rational.0 as u128))
        } else {
            cmp::Ordering::Equal //unknown
        }
    } else {
        cmp::Ordering::Equal //unknown
    }
}

#[inline]
pub(crate) fn compute_std_dev(bits: u8, window: usize, runner: u128, sum_sqr: u128) -> f32 {
    if runner < SQUARE_LIMIT {
        let r2 = (runner*runner)>>bits;
        //trace!("{} {} {} {} -> {} ",bits,window,runner,sum_sqr,r2);
        if sum_sqr > r2 {
            (((sum_sqr - r2) >> bits) as f32).sqrt() //TODO: 2025 someday we may need to implement sqrt for u128
        } else {
            ((sum_sqr as f32 / window as f32) - (runner as f32 / window as f32).powi(2)).sqrt()
        }
    } else {
        //trace!("{:?} {:?} {:?} {:?} " ,bits ,window,runner ,sum_sqr);
        ((sum_sqr as f32 / window as f32) - (runner as f32 / window as f32).powi(2)).sqrt()
    }
}

pub(crate) const SQUARE_LIMIT: u128 = (1 << 64)-1;


#[derive(Default,Debug)]
pub(crate) struct ChannelBlock<T> where T: Counter {
    pub(crate) histogram:      Option<Histogram<T>>,
    pub(crate) runner:         u128,
    pub(crate) sum_of_squares: u128,
}
