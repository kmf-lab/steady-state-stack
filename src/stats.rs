use std::cmp::Ordering;
use std::sync::Arc;
use std::time::Duration;
use log::{info, trace};
use num_traits::Zero;
use crate::{config, Filled, Percentile, Rate, StdDev, Trigger};
use crate::monitor::ChannelMetaData;

const DOT_RED: & 'static str = "red";
const DOT_GREEN: & 'static str = "green";
const DOT_YELLOW: & 'static str = "yellow";
const DOT_WHITE: & 'static str = "white";


static DOT_PEN_WIDTH: [&'static str; 16]
= ["1", "2", "3", "5", "8", "13", "21", "34", "55", "89", "144", "233", "377", "610", "987", "1597"];

const SQUARE_LIMIT: u128 = (1 << 64)-1; // (u128::MAX as f64).sqrt() as u128;

pub struct ChannelStatsComputer {
    pub(crate) display_labels: Option<Vec<& 'static str>>,
    pub(crate) line_expansion: bool,
    pub(crate) show_type: Option<& 'static str>,

    pub(crate) percentiles_inflight: Vec<Percentile>, //each is a row
    pub(crate) percentiles_consumed: Vec<Percentile>, //each is a row
    pub(crate) std_dev_inflight: Vec<StdDev>, //each is a row
    pub(crate) std_dev_consumed: Vec<StdDev>, //each is a row

    pub(crate) red_trigger: Vec<Trigger>, //if used base is green
    pub(crate) yellow_trigger: Vec<Trigger>, //if used base is green

    pub(crate) runner_inflight:u128,
    pub(crate) runner_consumed:u128,
    pub(crate) percentile_inflight:[u32;65],//based on u64.leading_zeros() we wil put the answers in that bucket
    pub(crate) percentile_consumed:[u32;65],//based on u64.leading_zeros() we wil put the answers in that bucket
    pub(crate) buckets_inflight:Vec<u64>,
    pub(crate) buckets_consumed:Vec<u64>,
    pub(crate) buckets_index:usize,
    pub(crate) buckets_mask:usize,
    pub(crate) buckets_bits:u8,
    pub(crate) frame_rate_ms:u128,

    pub(crate) time_label: String,
    pub(crate) prev_take: i128,
    pub(crate) has_full_window: bool,
    pub(crate) capacity: usize,

    pub(crate) sum_of_squares_inflight:u128,
    pub(crate) sum_of_squares_consumed:u128,

    pub(crate) show_avg_inflight:bool,
    pub(crate) show_avg_consumed:bool,
}

impl ChannelStatsComputer {

    pub(crate) fn empty() -> Self {
        ChannelStatsComputer {
            display_labels: None,
            line_expansion: false,
            show_type: None,
            percentiles_inflight: Vec::new(),
            percentiles_consumed: Vec::new(),
            std_dev_inflight: Vec::new(),
            std_dev_consumed: Vec::new(),
            red_trigger: Vec::new(),
            yellow_trigger: Vec::new(),
            runner_inflight:0,
            runner_consumed:0,
            percentile_inflight:[0u32;65],
            percentile_consumed:[0u32;65],
            buckets_inflight:Vec::new(),
            buckets_consumed:Vec::new(),
            buckets_index: usize::MAX,
            buckets_mask:0,
            buckets_bits:0,
            frame_rate_ms:0,
            time_label: String::new(),
            prev_take: 0,
            has_full_window: false,
            capacity: 0,
            sum_of_squares_inflight:0,
            sum_of_squares_consumed:0,
            show_avg_inflight:false,
            show_avg_consumed:false,
        }
    }

    pub(crate) fn init(&mut self, meta: &Arc<ChannelMetaData>)  {
        assert!(meta.capacity > 0, "capacity must be greater than 0");

        let display_labels = if meta.display_labels {
            Some(meta.labels.clone())
        } else {
            None
        };

        let buckets_bits      = meta.window_bucket_in_bits;
        let buckets_count:usize = 1<<meta.window_bucket_in_bits;
        let buckets_mask:usize = buckets_count - 1;
        let buckets_index:usize = 0;
        let time_label = time_label(buckets_count * config::TELEMETRY_PRODUCTION_RATE_MS);

        self.display_labels = display_labels;
        self.line_expansion = meta.line_expansion;
        self.show_type = meta.show_type;
        self.percentiles_inflight = meta.percentiles_inflight.clone();
        self.percentiles_consumed = meta.percentiles_consumed.clone();
        self.std_dev_inflight = meta.std_dev_inflight.clone();
        self.std_dev_consumed = meta.std_dev_consumed.clone();
        self.red_trigger = meta.red.clone();
        self.yellow_trigger = meta.yellow.clone();
        self.runner_inflight = 0;
        self.runner_consumed = 0;
        self.buckets_inflight = vec![0u64; buckets_count];
        self.buckets_consumed = vec![0u64; buckets_count];
        self.buckets_mask = buckets_mask;
        self.buckets_bits = buckets_bits;
        self.buckets_index = buckets_index;
        self.time_label = time_label;
        self.prev_take = 0i128;
        self.sum_of_squares_inflight = 0;
        self.sum_of_squares_consumed = 0;
        self.has_full_window = false;
        self.frame_rate_ms = config::TELEMETRY_PRODUCTION_RATE_MS as u128;
        self.capacity = meta.capacity;
        self.show_avg_inflight = meta.avg_inflight;
        self.show_avg_consumed = meta.avg_consumed;

    }

    pub(crate) fn compute(&mut self, send: i128, take: i128)
                          -> (String, & 'static str, & 'static str) {
        assert!(self.capacity > 0, "capacity must be greater than 0, this was probably not init");
        assert!(send>=take, "internal error send {} must be greater or eq than take {}",send,take);

        // compute the running totals

        let inflight:u64 = (send-take) as u64;
        let consumed:u64 = (take-self.prev_take) as u64;

        self.accumulate_data_frame(inflight, consumed);


        self.prev_take = take;

        //  build the label

        let mut display_label = String::new();

        self.show_type.map(|f| {
            display_label.push_str(f);
            display_label.push_str("\n");
        });

        if !self.buckets_bits.is_zero() {
            display_label.push_str("per ");
            display_label.push_str(&self.time_label);
            display_label.push_str("\n");
        }

        self.display_labels.as_ref().map(|labels| {
            //display_label.push_str("labels: ");
            labels.iter().for_each(|f| {
                display_label.push_str(f);
                display_label.push_str(" ");
            });
            display_label.push_str("\n");
        });

        //do not compute the std_devs/percentiles if we do not have a full window of data.
        if self.has_full_window && !self.buckets_bits.is_zero() {
            self.append_computed_labels(&mut display_label);
        }

        display_label.push_str("Capacity: ");
        display_label.push_str(itoa::Buffer::new().format(self.capacity));
        if !take.is_zero() { //only show when data starts getting sent
            display_label.push_str(" Total: ");
            display_label.push_str(itoa::Buffer::new().format(take));
        }
        display_label.push_str("\n");




        //set the default color in case we have no alerts.
        let mut line_color = DOT_WHITE;
        if  (!self.red_trigger.is_empty() || !self.yellow_trigger.is_empty())
            && self.has_full_window
            && !self.buckets_bits.is_zero() {
            line_color = DOT_GREEN;
            //Trigger colors now that we have the data
            //we can stop if we find a single trigger that is true
            if self.yellow_trigger.iter().any(|r| self.triggered(r)) {
                line_color = DOT_YELLOW;
            }
            if self.red_trigger.iter().any(|r| self.triggered(r)) {
                line_color = DOT_RED;
            }
        }


        let line_thick = if self.line_expansion {
            DOT_PEN_WIDTH[ (128usize-(take>>20).leading_zeros() as usize).min(DOT_PEN_WIDTH.len()-1) ]
        } else {
            "1"
        };

        self.has_full_window = self.has_full_window || self.buckets_index == 0;

        (display_label,line_color,line_thick)

    }

    fn append_computed_labels(&mut self, mut display_label: &mut String) {

        if self.show_avg_consumed {
            let avg = self.runner_consumed >> self.buckets_bits;
            display_label.push_str(format!("consumed avg: {:?} ", avg).as_str());
            display_label.push_str("\n");
        }

        if self.show_avg_inflight {
            let avg = self.runner_inflight >> self.buckets_bits;
            display_label.push_str(format!("inflight avg: {:?} ", avg).as_str());
            display_label.push_str("\n");
        }

        self.std_dev_inflight.iter().for_each(|f| {
            display_label.push_str(format!("inflight {:.1}StdDev: {:.1} "
                                           , f.value(), f.value() * self.inflight_std_dev()
            ).as_str());
            display_label.push_str("\n");
        });

        self.std_dev_consumed.iter().for_each(|f| {
            display_label.push_str(format!("consumed {:.1}StdDev: {:.1} "
                                           , f.value(), f.value() * self.consumed_std_dev()
            ).as_str());
            display_label.push_str("\n");
        });

        self.percentiles_consumed.iter().for_each(|p| {
            let window = 1u64 << self.buckets_bits;
            display_label.push_str(&*format!("consumed {:?}%ile {:?}"
                                   , (p.value() * 100f32) as usize
                                   , Self::compute_percentile_est(window, *p, self.percentile_consumed)));
            display_label.push_str("\n");
        });

        self.percentiles_inflight.iter().for_each(|p| {
            let window = 1u64 << self.buckets_bits;
            display_label.push_str(&*format!("inflight {:.1}%ile {:?}"
                                   , (p.value() * 100f32) as usize
                                   , Self::compute_percentile_est(window, *p, self.percentile_inflight)));
            display_label.push_str("\n");
        });

        //TODO: add 3 latency display options in the builder.

        //self.show_avg_latency

        //self.std_dev_latency.iter.for_each

        //self.percentiles_latency.iter.for_each

    }

    pub(crate) fn accumulate_data_frame(&mut self, inflight: u64, consumed: u64) {
        if usize::MAX != self.buckets_index {
            self.runner_inflight += inflight as u128;
            self.runner_inflight -= self.buckets_inflight[self.buckets_index] as u128;

            self.percentile_inflight[64 - inflight.leading_zeros() as usize] += 1u32;
            let old_zeros = self.buckets_inflight[self.buckets_index].leading_zeros() as usize;
            self.percentile_inflight[64 - old_zeros] = self.percentile_inflight[64 - old_zeros].saturating_sub(1u32);

            let inflight_square = (inflight as u128).pow(2);
            self.sum_of_squares_inflight = self.sum_of_squares_inflight.saturating_add(inflight_square);
            self.sum_of_squares_inflight = self.sum_of_squares_inflight.saturating_sub((self.buckets_inflight[self.buckets_index] as u128).pow(2));
            self.buckets_inflight[self.buckets_index] = inflight as u64;

            self.runner_consumed += consumed as u128;
            self.runner_consumed -= self.buckets_consumed[self.buckets_index] as u128;

            self.percentile_consumed[64 - consumed.leading_zeros() as usize] += 1;
            let old_zeros = self.buckets_consumed[self.buckets_index].leading_zeros() as usize;
            self.percentile_consumed[64 - old_zeros] = self.percentile_consumed[64 - old_zeros].saturating_sub(1);

            let consumed_square = (consumed as u128).pow(2);
            self.sum_of_squares_consumed = self.sum_of_squares_consumed.saturating_add(consumed_square);
            self.sum_of_squares_consumed = self.sum_of_squares_consumed.saturating_sub((self.buckets_consumed[self.buckets_index] as u128).pow(2));
            self.buckets_consumed[self.buckets_index] = consumed as u64;

            self.buckets_index = (1 + self.buckets_index) & self.buckets_mask;
        }
    }

    fn compute_percentile_est(window: u64, pct: Percentile, ptable: [u32; 65]) -> u128 {
        //TODO: this has room for improvement in the future
        let limit: u64 = (window as f32 * pct.value()) as u64;
        let mut walker = 0usize;
        let mut sum = 0u64;
        //info!("percentile compute:  window:{} pct: {} limit: {}",window, pct.value(), limit);
        while (walker < 62) && (sum + ptable[walker] as u64) < limit {
            //info!("  walker:{} sum:{} limit:{} ptable[walker]: {} ",walker, sum, limit, ptable[walker]);
            sum += ptable[walker] as u64;
            walker += 1;
        }
        if 62 == walker {
            //error handling roll backwards off any zeros, they did not help.
            while 0==ptable[walker] {
                walker -= 1;
            }
        }
        //info!("awn: {}  walker:{} sum:{} limit:{} ",1u128<<walker, walker, sum, limit);
        1u128<<walker
    }

    #[inline]
    fn compute_std_dev(bits: u8, window: usize, runner: u128, sum_sqr: u128) -> f32 {
            if runner < SQUARE_LIMIT {
                let r2 = (runner*runner)>>bits;
                //trace!("{} {} {} {} -> {} ",bits,window,runner,sum_sqr,r2);
                if sum_sqr > r2 {
                    (((sum_sqr - r2) >> bits) as f32).sqrt() //TODO: someday we may need to implement sqrt for u128
                } else {
                    ((sum_sqr as f32 / window as f32) - (runner as f32 / window as f32).powi(2)).sqrt()
                }
            } else {
                //trace!("{:?} {:?} {:?} {:?} " ,bits ,window,runner ,sum_sqr);
                ((sum_sqr as f32 / window as f32) - (runner as f32 / window as f32).powi(2)).sqrt()
            }
    }

    //TODO: triggers will be used for more than just color change see Alertmanager and Webhooks
    //TODO: other possible metric integrations  Prometheus, Grafana
    //      DataDog, Elastic Stack (ELK Stack: Elasticsearch, Logstash, and Kibana),
    //      New Relic, Splunk, AWS CloudWatch, Azure Monitor, Google Cloud Operations Suite

    //TODO: refactor to remove duplicate code

    ///  Rules, the triggers only act on the rolling data over the window against fixed limits
    ///  This is by design to reduce flicker and allow for readability of the results.
    ///  If you can not read the results then you should increase the window size.
    fn triggered(&self, rule: &Trigger) -> bool {
        match rule {
            Trigger::AvgFilledAbove(Filled::Percentage(percent_full_num, percent_full_den)) => {
                self.avg_filled_percentage(percent_full_num, percent_full_den).is_gt()
            }
            Trigger::AvgFilledBelow(Filled::Percentage(percent_full_num, percent_full_den)) => {
                self.avg_filled_percentage(percent_full_num, percent_full_den).is_lt()
            }
            Trigger::AvgFilledAbove(Filled::Exact(exact_full)) => {
                self.avg_filled_exact(exact_full).is_gt()
            }
            Trigger::AvgFilledBelow(Filled::Exact(exact_full)) => {
                self.avg_filled_exact(exact_full).is_lt()
            }
            Trigger::StdDevsFilledAbove(std_devs, Filled::Percentage(percent_full_num, percent_full_den)) => {
                self.stddev_filled_percentage(std_devs, percent_full_num, percent_full_den).is_gt()
            }
            Trigger::StdDevsFilledBelow(std_devs, Filled::Percentage(percent_full_num, percent_full_den)) => {
                self.stddev_filled_percentage(std_devs, percent_full_num, percent_full_den).is_lt()
            }
            Trigger::StdDevsFilledAbove(std_devs, Filled::Exact(exact_full)) => {
                self.stddev_filled_exact(std_devs, exact_full).is_gt()
            }
            Trigger::StdDevsFilledBelow(std_devs, Filled::Exact(exact_full)) => {
                self.stddev_filled_exact(std_devs, exact_full).is_lt()
            }
            Trigger::PercentileFilledAbove(percentile, Filled::Percentage(percent_full_num, percent_full_den)) => {
                self.percentile_filled_percentage(percentile, percent_full_num, percent_full_den).is_gt()
            }
            Trigger::PercentileFilledBelow(percentile, Filled::Percentage(percent_full_num, percent_full_den)) => {
                self.percentile_filled_percentage(percentile, percent_full_num, percent_full_den).is_lt()
            }
            Trigger::PercentileFilledAbove(percentile, Filled::Exact(exact_full)) => {
                self.percentile_filled_exact(percentile, exact_full).is_gt()
            }
            Trigger::PercentileFilledBelow(percentile, Filled::Exact(exact_full)) => {
                self.percentile_filled_exact(percentile, exact_full).is_lt()
            }
            /////////////////////////////////////////////////////////////////////////////////////
            /////////////////////////////////////////////////////////////////////////////////////
            // Rule: all rate values are in message per second so they can remain fixed while other values change

            Trigger::AvgRateBelow(rate) => {
                self.avg_rate(&rate).is_lt()
            },
            Trigger::AvgRateAbove(rate) => {
                self.avg_rate(&rate).is_gt()
            },
            Trigger::StdDevRateBelow(std_devs, rate) => {
                self.stddev_rate(std_devs, &rate).is_lt()
            }
            Trigger::StdDevRateAbove(std_devs, rate) => {
                self.stddev_rate(std_devs, &rate).is_gt()
            }
            Trigger::PercentileRateAbove(percentile, rate) => {
                self.percentile_rate(percentile, &rate).is_gt()
            }
            Trigger::PercentileRateBelow(percentile, rate) => {
                self.percentile_rate(percentile, &rate).is_lt()
            }
            ////////////////////////////////////////
            ////////////////////////////////////////

            Trigger::AvgLatencyAbove(duration) => {
                self.avg_latency(&duration).is_gt()
            }
            Trigger::AvgLatencyBelow(duration) => {
                self.avg_latency(&duration).is_lt()
            }
            Trigger::StdDevLatencyAbove(std_devs,duration) => {
                self.stddev_latency(std_devs, &duration).is_gt()
            }
            Trigger::StdDevLatencyBelow(std_devs,duration) => {
                self.stddev_latency(std_devs, &duration).is_lt()
            }
            Trigger::PercentileLatencyAbove(percentile, duration) => {
                self.percentile_latency(percentile, &duration).is_gt()
            }
            Trigger::PercentileLatencyBelow(percentile, duration) => {
                self.percentile_latency(percentile, &duration).is_lt()
            }

        }
    }

    #[inline]
    fn consumed_std_dev(&self) -> f32 {
        Self::compute_std_dev(self.buckets_bits
                              , 1 << self.buckets_bits
                              , self.runner_consumed
                              , self.sum_of_squares_consumed)
    }
    #[inline]
    fn inflight_std_dev(&self) -> f32 {
        Self::compute_std_dev(self.buckets_bits
                              , 1 << self.buckets_bits
                              , self.runner_inflight
                              , self.sum_of_squares_inflight)
    }
    fn percentile_latency(&self, percentile: &Percentile, duration: &Duration) -> Ordering {
        let consumed_rate = Self::compute_percentile_est(1 << self.buckets_bits as u32, *percentile, self.percentile_consumed);
        let in_flight = Self::compute_percentile_est(1 << self.buckets_bits as u32, *percentile, self.percentile_inflight);
        let unit_in_micros = 1000u128 * self.frame_rate_ms;
        (consumed_rate * unit_in_micros).cmp(&(duration.as_micros() * in_flight))
    }

    fn stddev_latency(&self, std_devs: &StdDev, duration: &Duration) -> Ordering {
        let avg_inflight = self.runner_inflight >> self.buckets_bits;
        let window_in_micros = 1000u128 * self.frame_rate_ms << self.buckets_bits;
        let adj_consumed_per_window = self.runner_consumed + ((self.consumed_std_dev() * std_devs.value()) as u128) << self.buckets_bits;
        let adj_inflight = avg_inflight + (self.inflight_std_dev() * std_devs.value()) as u128;
        (adj_inflight * window_in_micros).cmp(&(duration.as_micros() * adj_consumed_per_window))
    }

//TODO: we need to combine these with labels to test the values.

    fn avg_latency(&self, duration: &Duration) -> Ordering {
        let avg_inflight = self.runner_inflight >> self.buckets_bits;
        let window_in_micros = 1000u128 * self.frame_rate_ms << self.buckets_bits;
        //both sides * by runner_consumed to avoid the division
        (avg_inflight * window_in_micros).cmp(&(duration.as_micros() * self.runner_consumed))
    }

    fn percentile_rate(&self, percentile: &Percentile, rate: &Rate) -> Ordering {
        let measured_rate = Self::compute_percentile_est(1 << self.buckets_bits as u32, *percentile, self.percentile_consumed);
        let window_in_ms = (self.frame_rate_ms << self.buckets_bits) as u128;
        (measured_rate * 1000 * rate.to_rational_per_second().1 as u128).cmp(&(window_in_ms * rate.to_rational_per_second().0 as u128))
    }

    fn stddev_rate(&self, std_devs: &StdDev, rate: &Rate) -> Ordering {
        let std_deviation = ((self.consumed_std_dev() as f32 * std_devs.value()) * ((1 << self.buckets_bits) as f32)) as u128;
        let measured_rate_per_window = self.runner_consumed + std_deviation;
        let window_in_ms = self.frame_rate_ms << self.buckets_bits;
        (measured_rate_per_window * 1000 * rate.to_rational_per_second().1 as u128)
            .cmp(&(window_in_ms as u128 * rate.to_rational_per_second().0 as u128))
    }

    fn avg_rate(&self, rate: &Rate) -> Ordering {
        let window_in_ms = self.frame_rate_ms << self.buckets_bits;
        (self.runner_consumed * 1000 * rate.to_rational_per_second().1 as u128)
            .cmp(&(window_in_ms * rate.to_rational_per_second().0 as u128))
    }

    fn percentile_filled_exact(&self, percentile: &Percentile, exact_full: &u64) -> Ordering {
        let in_flight = Self::compute_percentile_est(1 << self.buckets_bits as u32, *percentile, self.percentile_inflight);
        in_flight.cmp(&(*exact_full as u128))
    }

    fn percentile_filled_percentage(&self, percentile: &Percentile, percent_full_num: &u64, percent_full_den: &u64) -> Ordering {
        let in_flight = Self::compute_percentile_est(1 << self.buckets_bits as u32, *percentile, self.percentile_inflight);
        (in_flight * *percent_full_den as u128).cmp(&(*percent_full_num as u128 * self.capacity as u128))
    }

    fn stddev_filled_exact(&self, std_devs: &StdDev, exact_full: &u64) -> Ordering {
        let std_deviation = (self.inflight_std_dev() * std_devs.value()) as u128;
        let avg = self.runner_inflight >> self.buckets_bits;
        // inflight >  avg + f*std
        (avg + std_deviation).cmp(&(*exact_full as u128))
    }

    fn stddev_filled_percentage(&self, std_devs: &StdDev, percent_full_num: &u64, percent_full_den: &u64) -> Ordering {
        let std_deviation = (self.inflight_std_dev() * std_devs.value()) as u128;
        let avg = self.runner_inflight >> self.buckets_bits;
        // inflight >  avg + f*std
        ((avg + std_deviation) * *percent_full_den as u128).cmp(&(*percent_full_num as u128 * self.capacity as u128))
    }



    fn avg_filled_percentage(&self, percent_full_num: &u64, percent_full_den: &u64) -> Ordering {
        (self.runner_inflight * *percent_full_den as u128).cmp(&((*percent_full_num as u128 * self.capacity as u128) << self.buckets_bits))
    }

    fn avg_filled_exact(&self, exact_full: &u64) -> Ordering {
        self.runner_inflight.cmp(&((*exact_full as u128) << self.buckets_bits as u128))
    }
}

fn time_label(total_ms: usize) -> String {
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
    } else {
        if seconds< 1.1 {"sec".to_string()} else {
            format!("{:.1} secs", seconds)
        }
    }
}

#[cfg(test)]
mod stats_tests {
    use super::*;
    use rand::prelude::*;
    use rand_distr::{Normal, Distribution};
    use std::sync::Arc;
    #[allow(unused_imports)]
    use log::*;


    fn build_start_computer() -> ChannelStatsComputer {
        let mut cmd = ChannelMetaData::default();

        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 4;
        let mut computer = ChannelStatsComputer::empty();
        computer.init(&Arc::new(cmd));
        computer.frame_rate_ms = 1; //each bucket is 1ms
        computer
    }


    #[test]
    fn test_avg_filled_above_percent_trigger() {
        let mut computer = build_start_computer();
        let c = 1<<computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame((computer.capacity as f32 * 0.81f32) as u64
                                           , 100);
        }
        // Define a trigger where the average filled above is 80%
        let trigger = Trigger::AvgFilledAbove(Filled::p80());
        assert!(computer.triggered(&trigger), "Trigger should fire when the average filled is above 80%");
    }

    #[test]
    fn test_avg_filled_below_percent_trigger() {
        let mut computer = build_start_computer();
        let c = 1<< computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame(( ((c as f32 * 0.02f32)+ 0.31f32)*computer.capacity as f32) as u64, 100);
        }
        assert!(computer.triggered(&Trigger::AvgFilledBelow(Filled::p70())), "Trigger should fire when the average filled is below 70%");
        assert!(!computer.triggered(&Trigger::AvgFilledBelow(Filled::p30())), "Trigger should not fire when the average filled is below 30%");
    }


    #[test]
    fn test_avg_filled_above_fixed_trigger() {
        let mut computer = build_start_computer();
        let c = 1<< computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame((computer.capacity as f32 * 0.81f32) as u64
                                           , 100);
        }
        assert!(computer.triggered(&Trigger::AvgFilledAbove(Filled::Exact(16))), "Trigger should fire when the average filled is above 16");
        assert!(!computer.triggered(&Trigger::AvgFilledAbove(Filled::Exact((computer.capacity - 1) as u64))), "Trigger should fire when the average filled is above 16");
    }

    #[test]
    fn test_avg_filled_below_fixed_trigger() {
        let mut computer = build_start_computer();
        let c = 1<< computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame(65, 100);
        }
        // Define a trigger where the average filled below is 60%
        assert!(computer.triggered(&Trigger::AvgFilledBelow(Filled::Exact(66))), "Trigger should fire when the average filled is below 66%");
        assert!(!computer.triggered(&Trigger::AvgFilledBelow(Filled::Exact(64))), "Trigger should not fire when the average filled is below 64%");

    }

    #[test]
    fn test_std_dev_filled_above_trigger() {
        let mut computer = build_start_computer();

        let mean = computer.capacity as f64 * 0.81; // mean value just above test
        let std_dev = 10.0; // standard deviation
        let normal = Normal::new(mean, std_dev).unwrap();
        let mut rng = thread_rng();

        let c = 1 << computer.buckets_bits;
        for _ in 0..c {
            let value = normal.sample(&mut rng).max(0.0).min(computer.capacity as f64) as u64;
             computer.accumulate_data_frame(value, 100);
        }

        // Define a trigger with a standard deviation condition
        assert!(computer.triggered(&Trigger::StdDevsFilledAbove(StdDev::one(), Filled::p80())), "Trigger should fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered(&Trigger::StdDevsFilledAbove(StdDev::one(), Filled::p90())), "Trigger should not fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered(&Trigger::StdDevsFilledAbove(StdDev::one(), Filled::p100())), "Trigger should not fire when standard deviation from the average filled is above the threshold");

    }

    #[test]
    fn test_std_dev_filled_below_trigger() {
        let mut computer = build_start_computer();

        let mean = computer.capacity as f64 * 0.61; // mean value just above test
        let std_dev = 10.0; // standard deviation
        let normal = Normal::new(mean, std_dev).unwrap();
        let mut rng = thread_rng();

        let c = 1 << computer.buckets_bits;
        for _ in 0..c {
            let value = normal.sample(&mut rng).max(0.0).min(computer.capacity as f64) as u64;
            computer.accumulate_data_frame(value, 100);
        }

        // Define a trigger with a standard deviation condition
        assert!(computer.triggered(&Trigger::StdDevsFilledBelow(StdDev::one(), Filled::p70())), "Trigger should fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered(&Trigger::StdDevsFilledBelow(StdDev::one(), Filled::p10())), "Trigger should NOT fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered(&Trigger::StdDevsFilledBelow(StdDev::one(), Filled::percentage(0.00f32).unwrap())), "Trigger should NOT fire when standard deviation from the average filled is above the threshold");

    }

    #[test]
    fn test_percentile_filled_above_trigger() {
        let mut computer = build_start_computer();
        // Simulate rate data accumulation
        let c = 1<<computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame(0,(5.0 * 1.2) as u64); // Simulating rate being consistently above a certain value
        }
        // Define a trigger for average rate above a threshold
  //TODO: fix      assert!(computer.triggered(&Trigger::PercentileFilledAbove(Percentile::p50(),Filled::Exact(20) )), "Trigger should fire when the average rate is above 5");
        //   assert!(!computer.triggered(&Trigger::AvgRateAbove(Rate::per_millis(197))), "Trigger should NOT fire when the average rate is above 20");
    }

    #[test]
    fn test_percentile_filled_below_trigger() {
        let mut computer = build_start_computer();
        // Simulate rate data accumulation
        let c = 1<<computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame(0,(2.0 * 0.8) as u64); // Simulating rate being consistently below a certain value
        }
        // Define a trigger for average rate below a threshold
        //     assert!(computer.triggered(&Trigger::AvgRateBelow(Rate::per_millis(3))), "Trigger should fire when the average rate is below 3");
        //TODO: fix   assert!(computer.triggered(&Trigger::PercentileFilledBelow(Percentile::p50(),Filled::Exact(20))), "Trigger should NOT fire when the average rate is below 7");
    }

    ////////////////////////////////////////////////////////
    #[test]
    fn test_avg_rate_above_trigger() {
        let mut computer = build_start_computer();
        // Simulate rate data accumulation
        let c = 1<<computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame(0,(5.0 * 1.2) as u64); // Simulating rate being consistently above a certain value
        }
        // Define a trigger for average rate above a threshold
        assert!(computer.triggered(&Trigger::AvgRateAbove(Rate::per_millis(5))), "Trigger should fire when the average rate is above 5");
     //   assert!(!computer.triggered(&Trigger::AvgRateAbove(Rate::per_millis(197))), "Trigger should NOT fire when the average rate is above 20");
    }

    #[test]
    fn test_avg_rate_below_trigger() {
        let mut computer = build_start_computer();
        // Simulate rate data accumulation
        let c = 1<<computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame(0,(2.0 * 0.8) as u64); // Simulating rate being consistently below a certain value
        }

        // Define a trigger for average rate below a threshold
   //     assert!(computer.triggered(&Trigger::AvgRateBelow(Rate::per_millis(3))), "Trigger should fire when the average rate is below 3");
        assert!(!computer.triggered(&Trigger::AvgRateBelow(Rate::per_millis(7))), "Trigger should NOT fire when the average rate is below 7");

    }

    #[test]
    fn test_std_dev_rate_above_trigger() {
        let mut computer = build_start_computer();
        let mean = 5.0; // mean rate
        let std_dev = 1.0; // standard deviation
        let normal = Normal::new(mean, std_dev).unwrap();
        let mut rng = thread_rng();

        // Simulate rate data with a distribution
        let c = 1 << computer.buckets_bits;
        for _ in 0..c {
            let value = normal.sample(&mut rng) as u64;
            computer.accumulate_data_frame(0,value);
        }

        // Define a trigger for rate deviation above a threshold
        assert!(computer.triggered(&Trigger::StdDevRateAbove(StdDev::one(), Rate::per_millis(4))), "Trigger should fire when rate deviates above the mean by a std dev, exceeding 6");
     //   assert!(!computer.triggered(&Trigger::StdDevRateAbove(StdDev::one(), Rate::per_millis(7))), "Trigger should NOT fire when rate deviates above the mean by a std dev, exceeding 7");

    }

    #[test]
    fn test_std_dev_rate_below_trigger() {
        let mut computer = build_start_computer();
        let mean = 3.0; // mean rate
        let std_dev = 0.5; // standard deviation
        let normal = Normal::new(mean, std_dev).unwrap();
        let mut rng = thread_rng();

        // Simulate rate data with a distribution
        let c = 1 << computer.buckets_bits;
        for _ in 0..c {
            let value = normal.sample(&mut rng) as u64;
            computer.accumulate_data_frame(0,value);
        }

        // Define a trigger for rate deviation below a threshold
    //    assert!(computer.triggered(&Trigger::StdDevRateBelow(StdDev::one(), Rate::per_millis(3))), "Trigger should fire when rate deviates below the mean by a std dev, not exceeding 3");
        assert!(!computer.triggered(&Trigger::StdDevRateBelow(StdDev::one(), Rate::per_millis(1))), "Trigger should NOT fire when rate deviates below the mean by a std dev, not exceeding 1");

    }
    #[test]
    fn test_percentile_rate_above_trigger() {
        let mut computer = build_start_computer();
        // Simulate rate data accumulation
        let c = 1<<computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame(0,(5.0 * 1.2) as u64); // Simulating rate being consistently above a certain value
        }
        // Define a trigger for average rate above a threshold
        assert!(computer.triggered(&Trigger::PercentileRateAbove(Percentile::p50(), Rate::per_millis(96 ) )), "Trigger should fire when the average rate is above 5");
        //   assert!(!computer.triggered(&Trigger::AvgRateAbove(Rate::per_millis(197))), "Trigger should NOT fire when the average rate is above 20");
    }

    #[test]
    fn test_percentile_rate_below_trigger() {
        let mut computer = build_start_computer();
        // Simulate rate data accumulation
        let c = 1<<computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame(0,(2.0 * 0.8) as u64); // Simulating rate being consistently below a certain value
        }
        // Define a trigger for average rate below a threshold
        //     assert!(computer.triggered(&Trigger::AvgRateBelow(Rate::per_millis(3))), "Trigger should fire when the average rate is below 3");
        assert!(!computer.triggered(&Trigger::PercentileRateBelow(Percentile::p50(), Rate::per_millis(96 ))), "Trigger should NOT fire when the average rate is below 7");
    }
/////////////////////////

#[test]
fn test_avg_latency_above_trigger() {
    let mut computer = build_start_computer();
    // Simulate rate data accumulation
    let c = 1<<computer.buckets_bits;
    for _ in 0..c { //TODO: fix the case when inflight is zero, we end up computeing latency wrong.
        computer.accumulate_data_frame(200,(5.0 * 3.2) as u64); // Simulating rate being consistently above a certain value
    }
    // Define a trigger for average rate above a threshold
    assert!(computer.triggered(&Trigger::AvgLatencyAbove( Duration::from_millis(1) )), "Trigger should fire when the average rate is above 5");
    //   assert!(!computer.triggered(&Trigger::AvgRateAbove(Rate::per_millis(197))), "Trigger should NOT fire when the average rate is above 20");
}

    #[test]
    fn test_avg_latency_below_trigger() {
        let mut computer = build_start_computer();
        // Simulate rate data accumulation
        let c = 1<<computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame(200,(2.0 * 0.8) as u64); // Simulating rate being consistently below a certain value
        }

        // Define a trigger for average rate below a threshold
        //     assert!(computer.triggered(&Trigger::AvgRateBelow(Rate::per_millis(3))), "Trigger should fire when the average rate is below 3");
        assert!(!computer.triggered(&Trigger::AvgLatencyBelow(Duration::from_millis(150))), "Trigger should NOT fire when the average rate is below 7");

    }

    #[test]
    fn test_std_dev_latency_above_trigger() {
        let mut computer = build_start_computer();
        let mean = 5.0; // mean rate
        let std_dev = 1.0; // standard deviation
        let normal = Normal::new(mean, std_dev).unwrap();
        let mut rng = thread_rng();

        // Simulate rate data with a distribution
        let c = 1 << computer.buckets_bits;
        for _ in 0..c {
            let value = normal.sample(&mut rng) as u64;
            computer.accumulate_data_frame(200,value);
        }

        // Define a trigger for rate deviation above a threshold
        assert!(computer.triggered(&Trigger::StdDevLatencyAbove(StdDev::one(), Duration::from_millis(1))), "Trigger should fire when rate deviates above the mean by a std dev, exceeding 6");
        //   assert!(!computer.triggered(&Trigger::StdDevRateAbove(StdDev::one(), Rate::per_millis(7))), "Trigger should NOT fire when rate deviates above the mean by a std dev, exceeding 7");

    }

    #[test]
    fn test_std_dev_latency_below_trigger() {
        let mut computer = build_start_computer();
        let mean = 3.0; // mean rate
        let std_dev = 0.5; // standard deviation
        let normal = Normal::new(mean, std_dev).unwrap();
        let mut rng = thread_rng();

        // Simulate rate data with a distribution
        let c = 1 << computer.buckets_bits;
        for _ in 0..c {
            let value = normal.sample(&mut rng) as u64;
            computer.accumulate_data_frame(30,value);
        }

        // Define a trigger for rate deviation below a threshold
        //    assert!(computer.triggered(&Trigger::StdDevRateBelow(StdDev::one(), Rate::per_millis(3))), "Trigger should fire when rate deviates below the mean by a std dev, not exceeding 3");
     //TODO:fix   assert!(!computer.triggered(&Trigger::StdDevLatencyBelow(StdDev::one(), Duration::from_millis(50000))), "Trigger should NOT fire when rate deviates below the mean by a std dev, not exceeding 1");

    }
    #[test]
    fn test_percentile_latency_above_trigger() {
        let mut computer = build_start_computer();
        // Simulate rate data accumulation
        let c = 1<<computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame(400,(5.0 * 1.2) as u64); // Simulating rate being consistently above a certain value
        }

        // Percentile, Rate

        // Define a trigger for average rate above a threshold
        //TODO:fix  assert!(computer.triggered(&Trigger::PercentileLatencyAbove(Percentile::p90(), Duration::from_millis(1) )), "Trigger should fire when the average rate is above 5");
        //   assert!(!computer.triggered(&Trigger::AvgRateAbove(Rate::per_millis(197))), "Trigger should NOT fire when the average rate is above 20");
    }

    #[test]
    fn test_percentile_latency_below_trigger() {
        let mut computer = build_start_computer();
        // Simulate rate data accumulation
        let c = 1<<computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame(20,(2.0 * 0.8) as u64); // Simulating rate being consistently below a certain value
        }

        // Define a trigger for average rate below a threshold
        //     assert!(computer.triggered(&Trigger::AvgRateBelow(Rate::per_millis(3))), "Trigger should fire when the average rate is below 3");
        //TODO:fix   assert!(!computer.triggered(&Trigger::PercentileLatencyBelow(Percentile::p80(), Duration::from_millis(5000))), "Trigger should NOT fire when the average rate is below 7");

    }



}
