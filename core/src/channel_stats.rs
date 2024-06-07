use std::cmp::Ordering;
use std::collections::VecDeque;
use std::process::exit;


#[allow(unused_imports)]
use log::*;
use num_traits::Zero;
use crate::*;
use hdrhistogram::{Counter, Histogram};
use itoa::Buffer;

use crate::actor_stats::ChannelBlock;


pub(crate) const DOT_GREEN: &str = "green";
pub(crate) const DOT_YELLOW: &str = "yellow";
pub(crate) const DOT_ORANGE: &str = "orange";
pub(crate) const DOT_RED: &str = "red";
pub(crate) const DOT_GREY: &str = "grey";


static DOT_PEN_WIDTH: [&str; 16]
= ["1", "2", "3", "5", "8", "13", "21", "34", "55", "89", "144", "233", "377", "610", "987", "1597"]; // (u128::MAX as f64).sqrt() as u128;

#[derive(Default,Debug)]
pub struct ChannelStatsComputer {
    pub(crate) display_labels: Option<Vec<& 'static str>>,
    pub(crate) line_expansion: bool,

    pub(crate) show_type: Option<& 'static str>,
    pub(crate) type_byte_count: usize, //used to know bytes/sec sent

    pub(crate) percentiles_filled: Vec<Percentile>, //to show
    pub(crate) percentiles_rate: Vec<Percentile>, //to show
    pub(crate) percentiles_latency: Vec<Percentile>, //to show

    pub(crate) std_dev_filled: Vec<StdDev>, //to show
    pub(crate) std_dev_rate: Vec<StdDev>, //to show
    pub(crate) std_dev_latency: Vec<StdDev>, //to show

    pub(crate) show_avg_filled:bool,
    pub(crate) show_avg_rate:bool,
    pub(crate) show_avg_latency:bool,
    pub(crate) show_max_filled:bool,  //TODO: show max filled
    pub(crate) show_min_filled:bool,  //TODO: show min filled

    pub(crate) rate_trigger: Vec<(Trigger<Rate>, AlertColor)>, //if used base is green
    pub(crate) filled_trigger: Vec<(Trigger<Filled>, AlertColor)>, //if used base is green
    pub(crate) latency_trigger: Vec<(Trigger<Duration>, AlertColor)>, //if used base is green

    // every one of these gets the sample, once the sample is full(N) we drop and add one.
    pub(crate) history_filled: VecDeque<ChannelBlock<u16>>, //biggest channel length is 64K?
    pub(crate) history_rate: VecDeque<ChannelBlock<u64>>,
    pub(crate) history_latency: VecDeque<ChannelBlock<u64>>,

    pub(crate) bucket_frames_count: usize, //when this bucket is full we add a new one
    pub(crate) refresh_rate_in_bits: u8,
    pub(crate) window_bucket_in_bits: u8,

    pub(crate) frame_rate_ms:u128, //const at runtime but needed here for unit testing
    pub(crate) time_label: String,
    pub(crate) prev_take: i128,
    pub(crate) capacity: usize,

    pub(crate) build_filled_histogram: bool,
    pub(crate) build_rate_histogram: bool,
    pub(crate) build_latency_histogram: bool,

    pub(crate) current_filled: Option<ChannelBlock<u16>>,
    pub(crate) current_rate: Option<ChannelBlock<u64>>,
    pub(crate) current_latency: Option<ChannelBlock<u64>>,

    pub(crate) prometheus_labels: String,
}

impl ChannelStatsComputer {

    pub(crate) fn init(&mut self, meta: &Arc<ChannelMetaData>, from_actor:usize, to_actor:usize, frame_rate_ms:u64)  {
        assert!(meta.capacity > 0, "capacity must be greater than 0");
        self.capacity = meta.capacity;

        self.prometheus_labels.push_str("channel_id=\"");
        self.prometheus_labels.push_str(itoa::Buffer::new().format(meta.id));
        self.prometheus_labels.push_str("\", ");
        self.prometheus_labels.push_str("actor_from_id=\"");
        self.prometheus_labels.push_str(itoa::Buffer::new().format(from_actor));
        self.prometheus_labels.push_str("\", ");
        self.prometheus_labels.push_str("actor_to_id=\"");
        self.prometheus_labels.push_str(itoa::Buffer::new().format(to_actor));
        self.prometheus_labels.push_str("\", ");



        //TODO: labels
        //self.prometheus_labels.push_str("actor_name=\"");
        //self.prometheus_labels.push_str(meta.labels); // "T"  for each one "F" for those filtered out
        //self.prometheus_labels.push_str("\", ");



        self.frame_rate_ms = frame_rate_ms as u128;
        self.refresh_rate_in_bits=meta.refresh_rate_in_bits;
        self.window_bucket_in_bits=meta.window_bucket_in_bits;
        self.time_label = actor_stats::time_label( self.frame_rate_ms << (meta.refresh_rate_in_bits+meta.window_bucket_in_bits) );


        self.display_labels = if meta.display_labels {
            Some(meta.labels.clone())
        } else {
            None
        };
        self.line_expansion = meta.line_expansion;
        self.show_type = meta.show_type;
        self.type_byte_count = meta.type_byte_count;

        self.show_avg_filled = meta.avg_filled;
        self.show_avg_rate = meta.avg_rate;
        self.show_avg_latency = meta.avg_latency;
        self.show_max_filled = meta.max_filled;
        self.show_min_filled = meta.min_filled;

        self.percentiles_filled.clone_from(&meta.percentiles_filled);
        self.percentiles_rate.clone_from(&meta.percentiles_rate);
        self.percentiles_latency.clone_from(&meta.percentiles_latency);

        self.std_dev_filled.clone_from(&meta.std_dev_inflight);
        self.std_dev_rate.clone_from(&meta.std_dev_consumed);
        self.std_dev_latency.clone_from(&meta.std_dev_latency);

        self.rate_trigger.clone_from(&meta.trigger_rate);
        self.filled_trigger.clone_from(&meta.trigger_filled);
        self.latency_trigger.clone_from(&meta.trigger_latency);

        //we set the build * histograms last after we bring in all the triggers

        let trigger_uses_histogram = self.filled_trigger.iter().any(|t|
            matches!(t, (Trigger::PercentileAbove(_,_),_) | (Trigger::PercentileBelow(_,_),_))
        );
        self.build_filled_histogram = trigger_uses_histogram || !self.percentiles_filled.is_empty();

        if self.build_filled_histogram {
            match Histogram::<u16>::new_with_bounds(1, self.capacity as u64, 0) {
                Ok(h) => {
                    self.history_filled.push_back(ChannelBlock {
                        histogram: Some(h),
                        runner: 0,
                        sum_of_squares: 0,
                    });
                }
                Err(e) => {
                    error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}"
                             ,self.capacity, 2, e);
                }
            }
        } else {
            self.history_filled.push_back(ChannelBlock::default());
        }

        let trigger_uses_histogram = self.rate_trigger.iter().any(|t|
            matches!(t, (Trigger::PercentileAbove(_,_),_) | (Trigger::PercentileBelow(_,_),_))
        );
        self.build_rate_histogram = trigger_uses_histogram || !self.percentiles_rate.is_empty();

        if self.build_rate_histogram {
            match Histogram::<u64>::new(2) {
                Ok(h) => {
                    self.history_rate.push_back(ChannelBlock {
                        histogram: Some(h),
                        runner: 0,
                        sum_of_squares: 0,
                    });
                }
                Err(e) => {
                    error!("unexpected, unable to create histogram of 2 sigfig err: {}", e);
                }
            }
        } else {
            self.history_rate.push_back(ChannelBlock::default());
        }


        let trigger_uses_histogram = self.latency_trigger.iter().any(|t|
            matches!(t, (Trigger::PercentileAbove(_,_),_) | (Trigger::PercentileBelow(_,_),_))
        );
        self.build_latency_histogram = trigger_uses_histogram || !self.percentiles_latency.is_empty();

        if self.build_latency_histogram {
            match Histogram::<u64>::new(2) {
                Ok(h) => {
                    self.history_latency.push_back(ChannelBlock {
                        histogram: Some(h),
                        runner: 0,
                        sum_of_squares: 0,
                    });
                }
                Err(e) => {
                    error!("unexpected, unable to create histogram of 2 sigfig err: {}", e);
                }
            }
        } else {
            self.history_latency.push_back(ChannelBlock::default());
        }
        self.prev_take = 0i128;
    }

    pub(crate) fn accumulate_data_frame(&mut self, filled: u64, rate: u64) {

            self.history_filled.iter_mut().for_each(|f| {
                 if let Some(ref mut h) = &mut f.histogram {
                     if let Err(e) = h.record(filled) {
                         error!("unexpected, unable to record filled {} err: {}", filled, e);
                     }
                 }
                 let filled:u64 = PLACES_TENS* filled;
                 f.runner = f.runner.saturating_add(filled as u128);
                 f.sum_of_squares = f.sum_of_squares.saturating_add((filled as u128).pow(2));
            });

            self.history_rate.iter_mut().for_each(|f| {
                if let Some(ref mut h) = &mut f.histogram {
                    if let Err(e) = h.record(rate) {
                        //histogram only does raw values
                        error!("unexpected, unable to record rate {} err: {}", rate, e);
                    }
                }
                let rate:u64 = PLACES_TENS* rate;
                f.runner = f.runner.saturating_add(rate as u128);
                f.sum_of_squares = f.sum_of_squares.saturating_add((rate as u128).pow(2));

            });


            self.history_latency.iter_mut().for_each(|f| {
                let latency_micros:u64 = if 0== rate {0u64}
                                            else {
                                                (filled * self.frame_rate_ms as u64) / rate
                                            };

                if let Some(ref mut h) = &mut f.histogram {
                    if let Err(e) = h.record(latency_micros) {
                        error!("unexpected, unable to record inflight {} err: {}", latency_micros, e);
                    }
                }

                let latency_micros:u64 = if 0== rate {0u64}
                                else {
                                    (filled * PLACES_TENS * self.frame_rate_ms as u64) / rate
                                };

                f.runner = f.runner.saturating_add(latency_micros as u128);
                f.sum_of_squares = f.sum_of_squares.saturating_add((latency_micros as u128).pow(2));

            });



            self.bucket_frames_count += 1;
            if self.bucket_frames_count >= (1<<self.refresh_rate_in_bits) {
                self.bucket_frames_count = 0;

                if self.history_filled.len() >= (1<<self.window_bucket_in_bits) {
                    self.current_filled = self.history_filled.pop_front();
                }
                if self.history_rate.len() >= (1<<self.window_bucket_in_bits) {
                    self.current_rate = self.history_rate.pop_front();
                }
                if self.history_latency.len() >= (1<<self.window_bucket_in_bits) {
                    self.current_latency = self.history_latency.pop_front();
                }

                if self.build_filled_histogram {
                    //if we can not create a histogram we act like the window is disabled and provide no data
                    match Histogram::<u16>::new_with_bounds(1, self.capacity as u64, 0) {
                        Ok(h) => {
                            self.history_filled.push_back(ChannelBlock {
                                histogram: Some(h), runner: 0, sum_of_squares: 0,
                            });
                        }
                        Err(e) => {
                            error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}"
                             ,self.capacity, 2, e);
                        }
                    }
                } else {
                    self.history_filled.push_back(ChannelBlock::default());
                }

                if self.build_rate_histogram {
                    //if we can not create a histogram we act like the window is disabled and provide no data
                    match Histogram::<u64>::new(2) {
                        Ok(h) => {
                            self.history_rate.push_back(ChannelBlock {
                                histogram: Some(h), runner: 0, sum_of_squares: 0,
                            });
                        }
                        Err(e) => {
                            error!("unexpected, unable to create histogram of 2 sigfig err: {}", e);
                        }
                    }
                } else {
                    self.history_rate.push_back(ChannelBlock::default());
                }

                if self.build_latency_histogram {
                    //if we can not create a histogram we act like the window is disabled and provide no data
                    match Histogram::<u64>::new(2) {
                        Ok(h) => {
                            self.history_latency.push_back(ChannelBlock {
                                histogram: Some(h), runner: 0, sum_of_squares: 0,
                            });
                        }
                        Err(e) => {
                            error!("unexpected, unable to create histogram of 2 sigfig err: {}", e);
                        }
                    }
                } else {
                    self.history_latency.push_back(ChannelBlock::default());
                }

            }
    }


    #[inline]
    fn rate_std_dev(&self) -> f32 {
        if let Some(c) = &self.current_rate {
            actor_stats::compute_std_dev(self.refresh_rate_in_bits + self.window_bucket_in_bits
                                         , 1 << (self.refresh_rate_in_bits + self.window_bucket_in_bits)
                                         , c.runner
                                         , c.sum_of_squares)
        } else {
            info!("skipping std no current data");
            0f32
        }
    }
    #[inline]
    fn filled_std_dev(&self) -> f32 {
        if let Some(c) = &self.current_filled {
            actor_stats::compute_std_dev(self.refresh_rate_in_bits + self.window_bucket_in_bits
                                         , 1 << (self.refresh_rate_in_bits + self.window_bucket_in_bits)
                                         , c.runner
                                         , c.sum_of_squares)
        } else {
            0f32
        }
    }

    #[inline]
    fn latency_std_dev(&self) -> f32 {
        if let Some(c) = &self.current_latency {
            actor_stats::compute_std_dev(self.refresh_rate_in_bits + self.window_bucket_in_bits
                                         , 1 << (self.refresh_rate_in_bits + self.window_bucket_in_bits)
                                         , c.runner
                                         , c.sum_of_squares)
        } else {
            0f32
        }
    }




    pub(crate) fn compute(&mut self
                          , mut display_label: &mut String
                          , mut metric_text: &mut String
                          , from_id: usize, send: i128, take: i128)
                          -> (& 'static str, & 'static str) {

        display_label.clear();
        metric_text.clear();

        if self.capacity==0 {
            return (DOT_GREY,"1");
        }
        assert!(self.capacity > 0, "capacity must be greater than 0 from actor {}, this was probably not init", from_id);

        // we are in a bad state just exit and give up
        #[cfg(debug_assertions)]
        if take>send { error!("actor {} take:{} is greater than send:{} ",from_id,take,send); exit(-1); }
        assert!(send>=take, "internal error send {} must be greater or eq than take {}",send,take);
        // compute the running totals

        let inflight:u64 = (send-take) as u64;
        let consumed:u64 = (take-self.prev_take) as u64;
        self.accumulate_data_frame(inflight, consumed);
        self.prev_take = take;

        ////////////////////////////////////////////////
        //  build the labels
        ////////////////////////////////////////////////

        self.show_type.iter().for_each(|f| {
            display_label.push_str(f);
            display_label.push('\n');
        });

        if !self.window_bucket_in_bits.is_zero() {
            display_label.push_str("Window ");
            display_label.push_str(&self.time_label);
            display_label.push('\n');
        }

        self.display_labels.as_ref().iter().for_each(|labels| {
            labels.iter().for_each(|f| {
                display_label.push_str(f);
                display_label.push(' ');
            });
            display_label.push('\n');
        });

        //does nothing if the value is None
        if let Some(ref current_rate) = self.current_rate {
            compute_labels(self.frame_rate_ms
                           , self.window_bucket_in_bits+self.refresh_rate_in_bits
                           , &mut display_label, current_rate
                                , "rate", "per/sec"
                                , u64::MAX
                                , (1000, self.frame_rate_ms as usize)
                                , self.show_avg_rate
                                , & self.std_dev_rate
                                , & self.percentiles_rate, &mut metric_text);
        }

        if let Some(ref current_filled) = self.current_filled {
            //info!("compute labels inflight: {:?}",self.std_dev_inflight);
            compute_labels(self.frame_rate_ms
                                , self.window_bucket_in_bits+self.refresh_rate_in_bits
                                , &mut display_label, current_filled
                                , "filled", "%"
                                , u64::MAX
                                , (100, self.capacity)
                                , self.show_avg_filled
                                , & self.std_dev_filled
                                , & self.percentiles_filled, &mut metric_text);
        }

        if let Some(ref current_latency) = self.current_latency {
            //info!("compute labels inflight: {:?}",self.std_dev_inflight);
            compute_labels(self.frame_rate_ms
                                , self.window_bucket_in_bits+self.refresh_rate_in_bits
                                , &mut display_label, current_latency
                                , "latency", "ms"
                                , u64::MAX
                                , (1,1)
                                , self.show_avg_latency
                                , & self.std_dev_latency
                                , & self.percentiles_latency, &mut metric_text);
        }

        display_label.push_str("Capacity: ");
        display_label.push_str(itoa::Buffer::new().format(self.capacity));
        display_label.push_str(" Total: ");
        display_label.push_str(itoa::Buffer::new().format(take));
        display_label.push('\n');


        //set the default color in case we have no alerts.
        let mut line_color = DOT_GREY;

        if self.trigger_alert_level(&AlertColor::Yellow) {
            line_color = DOT_YELLOW;
        };
        if self.trigger_alert_level(&AlertColor::Orange) {
            line_color = DOT_ORANGE;
        };
        if self.trigger_alert_level(&AlertColor::Red) {
            line_color = DOT_RED;
        };

        let line_thick = if self.line_expansion {
            DOT_PEN_WIDTH[ (128usize-(take>>20).leading_zeros() as usize).min(DOT_PEN_WIDTH.len()-1) ]
        } else {
            "1"
        };

        (line_color,line_thick)

    }

    fn trigger_alert_level(&mut self, c1: &AlertColor) -> bool {
        self.rate_trigger.iter().filter(|f| f.1.eq(c1)).any(|f| self.triggered_rate(&f.0))
        || self.filled_trigger.iter().filter(|f| f.1.eq(c1)).any(|f| self.triggered_filled(&f.0))
        || self.latency_trigger.iter().filter(|f| f.1.eq(c1)).any(|f| self.triggered_latency(&f.0))
    }

    fn triggered_latency(&self, rule: &Trigger<Duration>) -> bool {
        match rule {
            Trigger::AvgAbove(duration) => self.avg_latency(duration).is_gt(),
            Trigger::AvgBelow(duration) => self.avg_latency(duration).is_lt(),
            Trigger::StdDevsAbove(std_devs, duration) => self.stddev_latency(std_devs, duration).is_gt(),
            Trigger::StdDevsBelow(std_devs, duration) => self.stddev_latency(std_devs, duration).is_lt(),
            Trigger::PercentileAbove(percentile, duration) => self.percentile_latency(percentile, duration).is_gt(),
            Trigger::PercentileBelow(percentile, duration) => self.percentile_latency(percentile, duration).is_lt(),
        }
    }

    fn triggered_rate(&self, rule: &Trigger<Rate>) -> bool
    {
        match rule {
            Trigger::AvgBelow(rate) => {
                let window_in_ms = self.frame_rate_ms << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
                actor_stats::avg_rational(window_in_ms, &self.current_rate, rate.rational_ms()).is_lt()
            },
            Trigger::AvgAbove(rate) => {
                let window_in_ms = self.frame_rate_ms << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
                actor_stats::avg_rational(window_in_ms, &self.current_rate, rate.rational_ms()).is_gt()
            },
            Trigger::StdDevsBelow(std_devs, expected_rate) => {
                actor_stats::stddev_rational(self.rate_std_dev(), self.window_bucket_in_bits + self.refresh_rate_in_bits, std_devs
                                             , &self.current_rate, expected_rate.rational_ms()).is_lt()
            },
            Trigger::StdDevsAbove(std_devs, expected_rate) => {
                actor_stats::stddev_rational(self.rate_std_dev(), self.window_bucket_in_bits + self.refresh_rate_in_bits, std_devs
                                             , &self.current_rate, expected_rate.rational_ms()).is_gt()
            },
            Trigger::PercentileAbove(percentile, rate) => actor_stats::percentile_rational(percentile, &self.current_rate, rate.rational_ms()).is_gt(),
            Trigger::PercentileBelow(percentile, rate) => actor_stats::percentile_rational(percentile, &self.current_rate, rate.rational_ms()).is_lt(),
           }
    }

    fn triggered_filled(&self, rule: &Trigger<Filled>) -> bool
    {
        match rule {
            Trigger::AvgAbove(Filled::Percentage(percent_full_num, percent_full_den)) => self.avg_filled_percentage(percent_full_num, percent_full_den).is_gt(),
            Trigger::AvgBelow(Filled::Percentage(percent_full_num, percent_full_den)) => self.avg_filled_percentage(percent_full_num, percent_full_den).is_lt(),
            Trigger::AvgAbove(Filled::Exact(exact_full)) => self.avg_filled_exact(exact_full).is_gt(),
            Trigger::AvgBelow(Filled::Exact(exact_full)) => self.avg_filled_exact(exact_full).is_lt(),
            Trigger::StdDevsAbove(std_devs, Filled::Percentage(percent_full_num, percent_full_den)) => {
                self.stddev_filled_percentage(std_devs, percent_full_num, percent_full_den).is_gt()
            }
            Trigger::StdDevsBelow(std_devs, Filled::Percentage(percent_full_num, percent_full_den)) => {
                self.stddev_filled_percentage(std_devs, percent_full_num, percent_full_den).is_lt()
            }
            Trigger::StdDevsAbove(std_devs, Filled::Exact(exact_full)) => self.stddev_filled_exact(std_devs, exact_full).is_gt(),
            Trigger::StdDevsBelow(std_devs, Filled::Exact(exact_full)) => self.stddev_filled_exact(std_devs, exact_full).is_lt(),

            Trigger::PercentileAbove(percentile, Filled::Percentage(percent_full_num, percent_full_den)) => {
                self.percentile_filled_percentage(percentile, percent_full_num, percent_full_den).is_gt()
            }
            Trigger::PercentileBelow(percentile, Filled::Percentage(percent_full_num, percent_full_den)) => {
                self.percentile_filled_percentage(percentile, percent_full_num, percent_full_den).is_lt()
            }
            Trigger::PercentileAbove(percentile, Filled::Exact(exact_full)) => self.percentile_filled_exact(percentile, exact_full).is_gt(),
            Trigger::PercentileBelow(percentile, Filled::Exact(exact_full)) => self.percentile_filled_exact(percentile, exact_full).is_lt(),
        }
    }

    fn avg_filled_percentage(&self, percent_full_num: &u64, percent_full_den: &u64) -> Ordering {
        if let Some(current_inflight) = &self.current_filled {
            (current_inflight.runner * *percent_full_den as u128)
                .cmp(&((*percent_full_num as u128 * PLACES_TENS as u128 * self.capacity as u128)
                                  << (self.window_bucket_in_bits + self.refresh_rate_in_bits)))
        } else {
            Ordering::Equal //unknown
        }
    }

    fn avg_filled_exact(&self, exact_full: &u64) -> Ordering {
        if let Some(current_inflight) = &self.current_filled {
            current_inflight.runner
                .cmp(&((*exact_full as u128 * PLACES_TENS as u128)
                      << (self.window_bucket_in_bits + self.refresh_rate_in_bits)))
        } else {
            Ordering::Equal //unknown
        }
    }

    const MS_PER_SEC:u64 = 1000;
    fn avg_latency(&self, duration: &Duration) -> Ordering {
        if let Some(current_latency) = &self.current_latency {
            assert_eq!(Self::MS_PER_SEC, PLACES_TENS);//we assume this with as_micros below
            current_latency.runner
                .cmp(&((duration.as_micros())
                    << (self.window_bucket_in_bits + self.refresh_rate_in_bits)))
        } else {
            Ordering::Equal //unknown
        }
    }

    fn stddev_filled_exact(&self, std_devs: &StdDev, exact_full: &u64) -> Ordering {
        if let Some(current_inflight) = &self.current_filled {
            let std_deviation = (self.filled_std_dev() * std_devs.value()) as u128;
            let avg = current_inflight.runner >> (self.window_bucket_in_bits + self.refresh_rate_in_bits);
            // inflight >  avg + f*std
            (avg + std_deviation).cmp(&(*exact_full as u128 * PLACES_TENS as u128))
        } else {
            Ordering::Equal //unknown
        }

    }

    fn stddev_filled_percentage(&self, std_devs: &StdDev, percent_full_num: &u64, percent_full_den: &u64) -> Ordering {
        if let Some(current_inflight) = &self.current_filled {
            let std_deviation = (self.filled_std_dev() * std_devs.value()) as u128;
            let avg = current_inflight.runner >> (self.window_bucket_in_bits + self.refresh_rate_in_bits);
            // inflight >  avg + f*std
            ((avg + std_deviation) * *percent_full_den as u128).cmp(&(*percent_full_num as u128 * self.capacity as u128 * PLACES_TENS as u128))
         } else {
            Ordering::Equal //unknown
        }
    }

    fn stddev_latency(&self, std_devs: &StdDev, duration: &Duration) -> Ordering {
        if let Some(current_latency) = &self.current_latency {
            assert_eq!(1000, PLACES_TENS);//we assume this with as_micros below
            let std_deviation = (self.latency_std_dev() * std_devs.value()) as u128;
            let avg = current_latency.runner >> (self.window_bucket_in_bits + self.refresh_rate_in_bits);
            // inflight >  avg + f*std
            (avg + std_deviation).cmp(&(duration.as_micros()))
        } else {
            Ordering::Equal //unknown
        }
    }

    fn percentile_filled_exact(&self, percentile: &Percentile, exact_full: &u64) -> Ordering {
        if let Some(current_inflight) = &self.current_filled {
              if let Some(h) = &current_inflight.histogram {
                  let in_flight = h.value_at_percentile(percentile.percentile()) as u128;
                  in_flight.cmp(&(*exact_full as u128))
              } else {
                  Ordering::Equal //unknown
              }
        } else {
            Ordering::Equal //unknown
        }
    }

    fn percentile_filled_percentage(&self, percentile: &Percentile, percent_full_num: &u64, percent_full_den: &u64) -> Ordering {
        if let Some(current_inflight) = &self.current_filled {
            if let Some(h) = &current_inflight.histogram {
                let in_flight = h.value_at_percentile(percentile.percentile()) as u128;
                (in_flight * *percent_full_den as u128).cmp(&(*percent_full_num as u128 * self.capacity as u128))
            } else {
                Ordering::Equal //unknown
            }
        } else {
            Ordering::Equal //unknown
        }
    }

    fn percentile_latency(&self, percentile: &Percentile, duration: &Duration) -> Ordering {
        if let Some(current_latency) = &self.current_latency {
            if let Some(h) = &current_latency.histogram {
                let in_flight = h.value_at_percentile(percentile.percentile()) as u128;
                in_flight.cmp(&(duration.as_millis()))
            } else {
                Ordering::Equal //unknown
            }
        } else {
            Ordering::Equal //unknown
        }
    }
}

//#TODO: we need a new test to show the recovery from a panic

#[cfg(test)]
mod stats_tests {
    use super::*;
    use rand_distr::{Distribution, Normal};
    use rand::{rngs::StdRng, SeedableRng};
    use std::sync::Arc;
    #[allow(unused_imports)]
    use log::*;
    use crate::util;

    ////////////////////////////////
    // each of these tests cover both sides of above and below triggers with the matching label display
    ///////////////////////////////

    #[test]
    fn filled_avg_percent_trigger() {
        util::logger::initialize();

        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 2;
        cmd.refresh_rate_in_bits = 2;
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(cmd),1,2,42);
        computer.frame_rate_ms = 3;
        computer.show_avg_filled = true;

        let c = 1<<(computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {                     // 256 * 0.81 = 207
            computer.accumulate_data_frame((computer.capacity as f32 * 0.81f32) as u64
                                           , 100,
            );
        }

        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg filled: 80 %\n");
        assert!(computer.triggered_filled(&Trigger::AvgAbove(Filled::p80())), "Trigger should fire when the average filled is above");
        assert!(!computer.triggered_filled(&Trigger::AvgAbove(Filled::p90())), "Trigger should not fire when the average filled is above");
        assert!(!computer.triggered_filled(&Trigger::AvgBelow(Filled::p80())), "Trigger should not fire when the average filled is below");
        assert!(computer.triggered_filled(&Trigger::AvgBelow(Filled::p90())), "Trigger should fire when the average filled is below");

    }

    #[test]
    fn filled_avg_fixed_trigger() {
        util::logger::initialize();

        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 2;
        cmd.refresh_rate_in_bits = 2;
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(cmd),1,2,42);
        computer.frame_rate_ms = 3;
        computer.show_avg_filled = true;

        let c = 1<<(computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {                     // 256 * 0.81 = 207
            let filled = (computer.capacity as f32 * 0.81f32) as u64;
            let consumed = 100;
            computer.accumulate_data_frame(filled, consumed);
        }

        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg filled: 80 %\n");

        assert!(computer.triggered_filled(&Trigger::AvgAbove(Filled::Exact(16))), "Trigger should fire when the average filled is above");
        assert!(!computer.triggered_filled(&Trigger::AvgAbove(Filled::Exact((computer.capacity - 1) as u64))), "Trigger should not fire when the average filled is above");
        assert!(computer.triggered_filled(&Trigger::AvgBelow(Filled::Exact(220))), "Trigger should fire when the average filled is below");
        assert!(!computer.triggered_filled(&Trigger::AvgBelow(Filled::Exact(200))), "Trigger should not fire when the average filled is below");

    }


    #[test]
    fn filled_std_dev_trigger() {
        util::logger::initialize();

        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 2;
        cmd.refresh_rate_in_bits = 2;
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(cmd),1,2,42);
        computer.frame_rate_ms = 3;
        computer.show_avg_filled = true;

        let mean = computer.capacity as f64 * 0.81; // mean value just above test
        let expected_std_dev = 10.0; // standard deviation
        let normal = Normal::new(mean, expected_std_dev).unwrap();
        let seed = [42; 32];
        let mut rng = StdRng::from_seed(seed);

        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            let value = normal.sample(&mut rng).max(0.0).min(computer.capacity as f64) as u64;
             computer.accumulate_data_frame(value, 100, );
        }

        computer.std_dev_filled.push(StdDev::two_and_a_half());
        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg filled: 80 %\nfilled 2.5StdDev: 30.455 per frame (3ms duration)\n");

        computer.std_dev_filled.clear();
        computer.std_dev_filled.push(StdDev::one());
        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg filled: 80 %\nfilled StdDev: 12.182 per frame (3ms duration)\n");

        // Define a trigger with a standard deviation condition
        assert!(computer.triggered_filled(&Trigger::StdDevsAbove(StdDev::one(), Filled::p80())), "Trigger should fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_filled(&Trigger::StdDevsAbove(StdDev::one(), Filled::p90())), "Trigger should not fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_filled(&Trigger::StdDevsAbove(StdDev::one(), Filled::p100())), "Trigger should not fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_filled(&Trigger::StdDevsBelow(StdDev::one(), Filled::p80())), "Trigger should fire when standard deviation from the average filled is below the threshold");
        assert!(computer.triggered_filled(&Trigger::StdDevsBelow(StdDev::one(), Filled::p90())), "Trigger should not fire when standard deviation from the average filled is below the threshold");
        assert!(computer.triggered_filled(&Trigger::StdDevsBelow(StdDev::one(), Filled::p100())), "Trigger should not fire when standard deviation from the average filled is below the threshold");

    }

    #[test]
    fn filled_percentile_trigger() {
        util::logger::initialize();

        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 2;
        cmd.refresh_rate_in_bits = 2;

        cmd.percentiles_filled.push(Percentile::p25());
        cmd.percentiles_filled.push(Percentile::p50());
        cmd.percentiles_filled.push(Percentile::p75());
        cmd.percentiles_filled.push(Percentile::p90());

        let mut computer = ChannelStatsComputer::default();
        computer.frame_rate_ms = 3;
        assert!(computer.percentiles_filled.is_empty());
        computer.init(&Arc::new(cmd),1,2,42);
        assert!(!computer.percentiles_filled.is_empty());

        let mean = computer.capacity as f64 * 0.13;
        let expected_std_dev = 10.0; // standard deviation
        let normal = Normal::new(mean, expected_std_dev).unwrap();
        let seed = [42; 32];
        let mut rng = StdRng::from_seed(seed);

        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            let value = normal.sample(&mut rng).max(0.0).min(computer.capacity as f64) as u64;
            computer.accumulate_data_frame(value, 100, );
        }

        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "filled 25%ile 12 %\nfilled 50%ile 12 %\nfilled 75%ile 24 %\nfilled 90%ile 24 %\n");


        // Define a trigger with a standard deviation condition
        assert!(computer.triggered_filled(&Trigger::PercentileAbove(Percentile::p90(), Filled::Exact(47))), "Trigger should fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_filled(&Trigger::PercentileAbove(Percentile::p50(), Filled::p90())), "Trigger should not fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_filled(&Trigger::PercentileAbove(Percentile::p50(), Filled::p100())), "Trigger should not fire when standard deviation from the average filled is above the threshold");
        assert!(computer.triggered_filled(&Trigger::PercentileBelow(Percentile::p90(), Filled::Exact(80))), "Trigger should fire when standard deviation from the average filled is below the threshold");
        assert!(!computer.triggered_filled(&Trigger::PercentileBelow(Percentile::p50(), Filled::Exact(17))), "Trigger should not fire when standard deviation from the average filled is below the threshold");
        assert!(computer.triggered_filled(&Trigger::PercentileBelow(Percentile::p50(), Filled::p100())), "Trigger should fire when standard deviation from the average filled is below the threshold");

    }

    ////////////////////////////////////////////////////////
    #[test]
    fn rate_avg_trigger() {
        util::logger::initialize();

        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 2;
        cmd.refresh_rate_in_bits = 2;
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(cmd),1,2,42);
        computer.frame_rate_ms = 3;
        computer.show_avg_rate = true;

        //we consume 100 messages every computer.frame_rate_ms which is 3 ms
        //so per ms we are consuming about 33.3 messages

        let c = 1<<(computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            computer.accumulate_data_frame(0
                                           , 100,
            ); //frame rate is 1ms so 1000 per second
        }

        let display_label = compute_display_label(&mut computer);
        assert_eq!(display_label, "Avg rate: 33333 per/sec\n");

        //NOTE: our triggers are in fixed units so they do not need to change if we modify
        //      the frame rate, refresh rate or window rate.
        assert!(computer.triggered_rate(&Trigger::AvgAbove(Rate::per_millis(32))), "Trigger should fire when the average is above");
        assert!(!computer.triggered_rate(&Trigger::AvgAbove(Rate::per_millis(34))), "Trigger should not fire when the average is above");
        assert!(!computer.triggered_rate(&Trigger::AvgBelow(Rate::per_millis(32))), "Trigger should not fire when the average is below");
        assert!(computer.triggered_rate(&Trigger::AvgBelow(Rate::per_millis(34))), "Trigger should fire when the average is below");

    }


    #[test]
    fn rate_std_dev_trigger() {
        util::logger::initialize();

        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 2;
        cmd.refresh_rate_in_bits = 2;
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(cmd),1,2,42);
        computer.frame_rate_ms = 3;
        computer.show_avg_rate = true;

        let mean = computer.capacity as f64 * 0.81; // mean value just above test
        let expected_std_dev = 10.0; // standard deviation
        let normal = Normal::new(mean, expected_std_dev).unwrap();
        let seed = [42; 32];
        let mut rng = StdRng::from_seed(seed);

        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            let value = normal.sample(&mut rng).max(0.0).min(computer.capacity as f64) as u64;
            computer.accumulate_data_frame(100, value, );
        }

        computer.std_dev_rate.push(StdDev::two_and_a_half());
        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg rate: 68395 per/sec\nrate 2.5StdDev: 30.455 per frame (3ms duration)\n");

        computer.std_dev_rate.clear();
        computer.std_dev_rate.push(StdDev::one());
        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg rate: 68395 per/sec\nrate StdDev: 12.182 per frame (3ms duration)\n");

        // Define a trigger with a standard deviation condition
        assert!(computer.triggered_rate(&Trigger::StdDevsAbove(StdDev::one(), Rate::per_millis(80))), "Trigger should fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_rate(&Trigger::StdDevsAbove(StdDev::one(), Rate::per_millis(220))), "Trigger should not fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_rate(&Trigger::StdDevsBelow(StdDev::one(), Rate::per_millis(80))), "Trigger should fire when standard deviation from the average filled is below the threshold");
        assert!(computer.triggered_rate(&Trigger::StdDevsBelow(StdDev::one(), Rate::per_millis(220))), "Trigger should not fire when standard deviation from the average filled is below the threshold");

    }

    ///////////////////
    #[test]
    fn rate_percentile_trigger() {
        util::logger::initialize();

        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 2;
        cmd.refresh_rate_in_bits = 2;
        cmd.percentiles_rate.push(Percentile::p25());
        cmd.percentiles_rate.push(Percentile::p50());
        cmd.percentiles_rate.push(Percentile::p75());
        cmd.percentiles_rate.push(Percentile::p90());
        let mut computer = ChannelStatsComputer::default();
        computer.frame_rate_ms = 3;
        computer.init(&Arc::new(cmd),1,2,42);

        let mean = computer.capacity as f64 * 0.13;
        let expected_std_dev = 10.0; // standard deviation
        let normal = Normal::new(mean, expected_std_dev).unwrap();
        let seed = [42; 32];
        let mut rng = StdRng::from_seed(seed);

        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            let value = normal.sample(&mut rng).max(0.0).min(computer.capacity as f64) as u64;
            computer.accumulate_data_frame(100, value, );
        }

        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "rate 25%ile 595 per/sec\nrate 50%ile 666 per/sec\nrate 75%ile 904 per/sec\nrate 90%ile 1166 per/sec\n");

        // Define a trigger with a standard deviation condition
        assert!(computer.triggered_rate(&Trigger::PercentileAbove(Percentile::p90(), Rate::per_millis(47))), "Trigger should fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_rate(&Trigger::PercentileAbove(Percentile::p90(), Rate::per_millis(52))), "Trigger should not fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_rate(&Trigger::PercentileBelow(Percentile::p90(), Rate::per_millis(47))), "Trigger should fire when standard deviation from the average filled is above the threshold");
        assert!(computer.triggered_rate(&Trigger::PercentileBelow(Percentile::p90(), Rate::per_millis(52))), "Trigger should not fire when standard deviation from the average filled is above the threshold");

    }

    fn compute_display_label(computer: &mut ChannelStatsComputer) -> String {
        let mut display_label = String::new();
        let mut metrics = String::new();
        if let Some(ref current_rate) = computer.current_rate {
            compute_rate_labels(&computer, &mut display_label, &mut metrics, &current_rate);
        }
        if let Some(ref current_filled) = computer.current_filled {
            compute_filled_labels(&computer, &mut display_label, &mut metrics, &current_filled);
        }
        if let Some(ref current_latency) = computer.current_latency {
            compute_latency_labels(&computer, &mut display_label, &mut metrics, &current_latency);
        }
        display_label
    }

/////////////////////////

#[test]
fn latency_avg_trigger() {
    util::logger::initialize();

    let mut cmd = ChannelMetaData::default();
    cmd.capacity = 256;
    cmd.window_bucket_in_bits = 2;
    cmd.refresh_rate_in_bits = 2;
    let mut computer = ChannelStatsComputer::default();
    computer.init(&Arc::new(cmd),1,2,42);
    computer.frame_rate_ms = 3;
    computer.show_avg_latency = true;

    let mean = computer.capacity as f64 * 0.81; // mean value just above test
    let expected_std_dev = 10.0; // standard deviation
    let normal = Normal::new(mean, expected_std_dev).unwrap();
    let seed = [42; 32];
    let mut rng = StdRng::from_seed(seed);

    let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
    for _ in 0..c {
        // 205 in flight and we consume 33 per 3ms frame 11 per ms, so 205/11 = 18.6ms
        let inflight_value = normal.sample(&mut rng).max(0.0).min(computer.capacity as f64) as u64;
        computer.accumulate_data_frame(inflight_value, 33, );
    }

    let display_label = compute_display_label(&mut computer);

    assert_eq!(display_label, "Avg latency: 18 ms\n");

    assert!(computer.triggered_latency(&Trigger::AvgAbove(Duration::from_millis(5))), "Trigger should fire when the average is above");
    assert!(!computer.triggered_latency(&Trigger::AvgAbove(Duration::from_millis(21))), "Trigger should fire when the average is above");

    assert!(!computer.triggered_latency(&Trigger::AvgBelow(Duration::from_millis(5))), "Trigger should fire when the average is above");
    assert!(computer.triggered_latency(&Trigger::AvgBelow(Duration::from_millis(21))), "Trigger should fire when the average is above");

}

fn compute_rate_labels(computer: &ChannelStatsComputer, target_telemetry_label: &mut String, target_metric: &mut String, current_rate: &&ChannelBlock<u64>) {
    compute_labels(computer.frame_rate_ms
                   , computer.window_bucket_in_bits + computer.refresh_rate_in_bits
                   , target_telemetry_label, &current_rate
                   , &"rate", &"per/sec"
                   , u64::MAX
                   , (1000, computer.frame_rate_ms as usize)
                   , computer.show_avg_rate
                   , &computer.std_dev_rate
                   , &computer.percentiles_rate
                   , target_metric);
}

    fn compute_filled_labels(computer: &ChannelStatsComputer, display_label: &mut String, metric_target: &mut String, current_filled: &&ChannelBlock<u16>) {
    compute_labels(computer.frame_rate_ms
                   , computer.window_bucket_in_bits + computer.refresh_rate_in_bits
                   , display_label, &current_filled
                   , &"filled", &"%"
                   , u64::MAX
                   , (100, computer.capacity)
                   , computer.show_avg_filled
                   , &computer.std_dev_filled
                   , &computer.percentiles_filled
                   , metric_target);
}

    fn compute_latency_labels(computer: &ChannelStatsComputer, display_label: &mut String, metric_target: &mut String, current_latency: &&ChannelBlock<u64>) {
    compute_labels(computer.frame_rate_ms
                   , computer.window_bucket_in_bits + computer.refresh_rate_in_bits
                   , display_label, &current_latency
                   , &"latency", &"ms"
                   , u64::MAX
                   , (1, 1)
                   , computer.show_avg_latency
                   , &computer.std_dev_latency
                   , &computer.percentiles_latency
                   , metric_target);
}


    #[test]
    fn latency_std_dev_trigger() {
        util::logger::initialize();

        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 2;
        cmd.refresh_rate_in_bits = 2;
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(cmd),1,2,42);
        computer.frame_rate_ms = 3;
        computer.show_avg_latency = true;
        let mean = 5.0; // mean rate
        let std_dev = 1.0; // standard deviation
        let normal = Normal::new(mean, std_dev).unwrap();
        let seed = [42; 32];
        let mut rng = StdRng::from_seed(seed);

        // Simulate rate data with a distribution
        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            let consumed_value = normal.sample(&mut rng) as u64;
            computer.accumulate_data_frame(computer.capacity as u64 /2, consumed_value, );
        }

        computer.std_dev_latency.push(StdDev::two_and_a_half());
        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg latency: 95 ms\nlatency 2.5StdDev: 79.329 per frame (3ms duration)\n");

        // Define a trigger for rate deviation above a threshold
        assert!(computer.triggered_latency(&Trigger::StdDevsAbove(StdDev::two_and_a_half(), Duration::from_millis(96+70))), "Trigger should fire when rate deviates above the mean by a std dev, exceeding 6");
        assert!(!computer.triggered_latency(&Trigger::StdDevsAbove(StdDev::two_and_a_half(), Duration::from_millis(96+90))), "Trigger should not fire when rate deviates above the mean by a std dev, exceeding 6");
        assert!(!computer.triggered_latency(&Trigger::StdDevsBelow(StdDev::two_and_a_half(), Duration::from_millis(96+70))), "Trigger should not fire when rate deviates above the mean by a std dev, exceeding 6");
        assert!(computer.triggered_latency(&Trigger::StdDevsBelow(StdDev::two_and_a_half(), Duration::from_millis(96+90))), "Trigger should fire when rate deviates above the mean by a std dev, exceeding 6");

    }

    #[test]
    fn latency_percentile_trigger() {
        util::logger::initialize();

        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 2;
        cmd.refresh_rate_in_bits = 2;
        cmd.percentiles_latency.push(Percentile::p25());
        cmd.percentiles_latency.push(Percentile::p50());
        cmd.percentiles_latency.push(Percentile::p75());
        cmd.percentiles_latency.push(Percentile::p90());
        let mut computer = ChannelStatsComputer::default();
        computer.frame_rate_ms = 3;
        computer.init(&Arc::new(cmd),1,2,42);

        // Simulate rate data accumulation
        let c = 1<<(computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            computer.accumulate_data_frame((computer.capacity-1) as u64, (5.0 * 1.2) as u64, ); // Simulating rate being consistently above a certain value
        }

        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "latency 25%ile 1791 ms\nlatency 50%ile 1791 ms\nlatency 75%ile 1791 ms\nlatency 90%ile 1791 ms\n");

        // Define a trigger for average rate above a threshold
        assert!(computer.triggered_latency(&Trigger::PercentileAbove(Percentile::p90(), Duration::from_millis(100) )), "Trigger should fire when the average rate is above");
        assert!(!computer.triggered_latency(&Trigger::PercentileAbove(Percentile::p90(), Duration::from_millis(2000) )), "Trigger should fire when the average rate is above");
        assert!(!computer.triggered_latency(&Trigger::PercentileBelow(Percentile::p90(), Duration::from_millis(100) )), "Trigger should fire when the average rate is below");
        assert!(computer.triggered_latency(&Trigger::PercentileBelow(Percentile::p90(), Duration::from_millis(2000) )), "Trigger should fire when the average rate is below");


    }

}

pub(crate) const PLACES_TENS:u64 = 1000u64;

#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_labels<T: Counter>(frame_rate_ms: u128
                                         , window_in_bits: u8
                                         , label_target: &mut String
                                         , current: &ChannelBlock<T>
                                         , label: &str, unit: &str
                                         , max_value: u64
                                         , rational_adjust: (usize,usize)
                                         , show_avg: bool
                                         , std_dev: &[StdDev]
                                         , percentile: &[Percentile]
                                         , metric_target: &mut String) {

    //we have frames, refresh rates and windows
    //  a frame is a single slice of time, viewable
    //  a refresh rate is how many frames we remain unchanged before showing new derived fields
    //         for example we want to show the moving averages slow enough for humans to read it.
    //  a window is a count of refresh rates, this is the time we use to compute the averages and std_devs
    //  this solution is designed to be able to show the data in a human readable way

    // a typical frame rate is 100ms (always greater than 32ms)
    // a typical refresh rate is 100+ frames or every 10 seconds of frames
    // a typical window is 6 or more refreshes for a 1 minute moving average

    //over the window
    //Average latency: 18.652 ms   (custom unit)
    //Average rate: 2000 per/second  (only one converted, and custom unit)
    //Average fill:  units
    //Average mcpu: 256
    //Average work: 25%          (custom unit)


    if show_avg {
        // info!("{} runner: {:?} shift: {:?}",label, current.runner, (self.refresh_rate_in_bits + self.window_bucket_in_bits));
        label_target.push_str("Avg ");
        label_target.push_str(label);

        metric_target.push_str("avg_");
        metric_target.push_str(label);
      //  metric_target.push_str("{nodeid=\""); //TODO: pass in nodeid???
      //  label_target.push_str(itoa::Buffer::new().format(self.id));
       // metric_target.push_str("\"} ");

        let denominator = PLACES_TENS * rational_adjust.1 as u64;
        let avg_per_sec_numer = (rational_adjust.0 as u128 *current.runner) >> window_in_bits;
        if avg_per_sec_numer >= (10 * denominator) as u128 {
            let value = avg_per_sec_numer / denominator as u128;

            metric_target.push_str(itoa::Buffer::new().format(value));

            if value >= 500000 {
                label_target.push_str(": ");
                label_target.push_str(itoa::Buffer::new().format(value/1000));
                label_target.push('k');
            } else {
                label_target.push_str(": ");
                label_target.push_str(itoa::Buffer::new().format(value));
            }
        } else {
            let value = avg_per_sec_numer as f32 / denominator as f32;
            metric_target.push_str(&format!(": {:.3}", value));

            label_target.push_str(&format!(": {:.3}", value));
        }
        metric_target.push_str("\n");


        label_target.push(' ');
        label_target.push_str(unit);
        label_target.push('\n');
    }

    let std = if !std_dev.is_empty() {
        actor_stats::compute_std_dev(window_in_bits
                                     , 1 << window_in_bits
                                     , current.runner
                                     , current.sum_of_squares)

    } else { 0f32 };
    std_dev.iter().for_each(|f| {

        // Inflight Operations 2.5 Standard Deviations Above Mean: 30455.1 events in 3 ms Frame

        label_target.push_str(label);
        label_target.push(' ');
        if *f != StdDev::one() {
            label_target.push_str(&format!("{:.1}", f.value()) );
        }
        label_target.push_str("StdDev: "); //this is per frame.
        label_target.push_str(
            &format!("{:.3}", (f.value() * std) / PLACES_TENS as f32));

        label_target.push_str(" per frame (");
        label_target.push_str(Buffer::new().format(frame_rate_ms));
        label_target.push_str("ms duration)\n");
    });

    percentile.iter().for_each(|p| {
        label_target.push_str(label);
        label_target.push(' ');

        label_target.push_str(Buffer::new().format(p.percentile() as usize));
        label_target.push_str("%ile ");

        if let Some(h) = &current.histogram {

             let value = ((rational_adjust.0 as f32 * h.value_at_percentile(p.percentile()).min(max_value)  as f32)
                          / rational_adjust.1 as f32) as usize;

            label_target.push_str(Buffer::new().format(value));

        } else {
            label_target.push_str("InternalError"); //not expected to happen
            error!("InternalError: no histogram for required percentile {:?}",p);
        }
        label_target.push(' ');
        label_target.push_str(unit);
        label_target.push('\n');
    });
}
