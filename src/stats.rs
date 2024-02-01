use std::sync::Arc;
use num_traits::Zero;
use crate::{config, Filled, Percentile, StdDev, Trigger};
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

    pub(crate) fn new(meta: &Arc<ChannelMetaData>) -> ChannelStatsComputer {

        let display_labels = if meta.display_labels {
            Some(meta.labels.clone())
        } else {
            None
        };
        let line_expansion = meta.line_expansion;
        let show_type = meta.show_type;

        let red_trigger                       = meta.red.clone();
        let yellow_trigger                    = meta.yellow.clone();

        let buckets_bits = meta.window_bucket_in_bits;
        let buckets_count:usize = 1<<meta.window_bucket_in_bits;
        let buckets_mask:usize = buckets_count - 1;
        let buckets_index:usize = 0;
        let time_label = time_label(buckets_count * config::TELEMETRY_PRODUCTION_RATE_MS);

        let buckets_inflight:Vec<u64> = vec![0u64; buckets_count];
        let buckets_consumed:Vec<u64> = vec![0u64; buckets_count];
        let runner_inflight:u128      = 0;
        let runner_consumed:u128      = 0;
        let percentile_inflight       = [0u32;65];
        let percentile_consumed       = [0u32;65];

        let  prev_take = 0i128;

        let sum_of_squares_inflight:u128= 0;
        let sum_of_squares_consumed:u128= 0;
        let capacity = meta.capacity;
        let show_avg_inflight = meta.avg_inflight;
        let show_avg_consumed = meta.avg_consumed;



        ChannelStatsComputer {
            display_labels,
            line_expansion,
            show_type,
            percentiles_inflight: meta.percentiles_inflight.clone(),
            percentiles_consumed: meta.percentiles_consumed.clone(),
            std_dev_inflight: meta.std_dev_inflight.clone(),
            std_dev_consumed: meta.std_dev_consumed.clone(),
            red_trigger,
            yellow_trigger,
            runner_inflight,
            runner_consumed,
            percentile_inflight,
            percentile_consumed,
            buckets_inflight,
            buckets_consumed,
            buckets_mask,
            buckets_bits,
            buckets_index,
            time_label,
            prev_take,
            sum_of_squares_inflight,
            sum_of_squares_consumed,
            has_full_window : false,
            capacity,
            show_avg_inflight,
            show_avg_consumed,

        }
    }

    pub(crate) fn compute(&mut self, send: i128, take: i128)
                          -> (String, & 'static str, & 'static str) {

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

        //set the default color in case we have no alerts.
        let mut line_color = if self.red_trigger.is_empty()
                         && self.yellow_trigger.is_empty() {
                            DOT_WHITE
                        } else {
                            DOT_GREEN
                        };


        //do not compute the std_devs if we do not have a full window of data.
        if self.has_full_window && !self.buckets_bits.is_zero() {
            let window = 1usize<<self.buckets_bits;
            if self.show_avg_consumed {
                let avg = self.runner_consumed as f64 / window as f64;
                display_label.push_str(format!("consumed avg: {:.1} ", avg).as_str());
                display_label.push_str("\n");
            }

            if self.show_avg_inflight {
                let avg = self.runner_inflight as f64 / window as f64;
                display_label.push_str(format!("inflight avg: {:.1} ", avg).as_str());
                display_label.push_str("\n");
            }

            self.std_dev_inflight.iter().for_each(|f| {
                let (label,mult,runner,sum_sqr) = {
                    //display_label.push_str("InFlight ");
                    let runner = self.runner_inflight;
                    let sum_sqr = self.sum_of_squares_inflight;
                    ("inflight", f.value(), runner, sum_sqr)
                };
                let std_deviation = Self::compute_std_dev(self.buckets_bits, window, runner, sum_sqr);
                display_label.push_str(format!("{} {:.1}StdDev: {:.1} "
                                               , label,mult, mult*std_deviation as f32
                                               ).as_str());
                display_label.push_str("\n");
            });

            self.std_dev_consumed.iter().for_each(|f| {
                let (label,mult,runner,sum_sqr) = {
                    //display_label.push_str("Consumed ");
                    let runner = self.runner_consumed;
                    let sum_sqr = self.sum_of_squares_consumed;
                    ("consumed",f.value(),runner,sum_sqr)
                };
                let std_deviation = Self::compute_std_dev(self.buckets_bits, window, runner, sum_sqr);
                display_label.push_str(format!("{} {:.1}StdDev: {:.1} "
                                               , label,mult, mult*std_deviation as f32
                ).as_str());
                display_label.push_str("\n");
            });

            if self.percentiles_consumed.len() > 0 {
               // display_label.push_str("Percentiles\n");
                self.percentiles_consumed.iter().for_each(|p| {
                    let (pct, label, percentile) = {
                            let percentile = Self::compute_percentile_est(window as u32, p.value(), self.percentile_consumed);
                            (p.value(), "consumed", percentile)
                         };
                    display_label.push_str(&*format!("{} {:?}%ile {:?}"
                                                     , label, pct * 100f32
                                                     , percentile));

                });
                display_label.push_str("\n");
            }

            if self.percentiles_inflight.len() > 0 {
                // display_label.push_str("Percentiles\n");
                self.percentiles_inflight.iter().for_each(|p| {
                    let (pct, label, percentile) ={
                            let percentile = Self::compute_percentile_est(window as u32, p.value(), self.percentile_inflight);
                            (p.value(), "inflight", percentile)
                    };
                    display_label.push_str(&*format!("{} {:?}%ile {:?}"
                                                     , label, pct * 100f32
                                                     , percentile));
                });
                display_label.push_str("\n");
            }


            //TODO: add latency display as an option in the builder.

            //Trigger colors now that we have the data
            self.yellow_trigger.iter().for_each(|t| {
                if self.triggered(t) {
                    line_color = DOT_YELLOW;
                }
            });
            self.red_trigger.iter().for_each(|t| {
                if self.triggered(t) {
                    line_color = DOT_RED;
                }
            });

        }

        display_label.push_str("Capacity: ");
        display_label.push_str(itoa::Buffer::new().format(self.capacity));
        if !take.is_zero() { //only show when data starts getting sent
            display_label.push_str(" Total: ");
            display_label.push_str(itoa::Buffer::new().format(take));
        }
        display_label.push_str("\n");

        let line_thick = if self.line_expansion {
            DOT_PEN_WIDTH[ (128usize-(take>>20).leading_zeros() as usize).min(DOT_PEN_WIDTH.len()-1) ]
        } else {
            "1"
        };

        self.has_full_window = self.has_full_window || self.buckets_index == 0;

        (display_label,line_color,line_thick)

    }

    pub(crate) fn accumulate_data_frame(&mut self, inflight: u64, consumed: u64) {
        self.runner_inflight += inflight as u128;
        self.runner_inflight -= self.buckets_inflight[self.buckets_index] as u128;

        self.percentile_inflight[64 - inflight.leading_zeros() as usize] += 1u32;
        let old_zeros = self.buckets_inflight[self.buckets_index].leading_zeros() as usize;
        self.percentile_inflight[old_zeros] = self.percentile_inflight[64 - old_zeros].saturating_sub(1u32);

        let inflight_square = (inflight as u128).pow(2);
        self.sum_of_squares_inflight = self.sum_of_squares_inflight.saturating_add(inflight_square);
        self.sum_of_squares_inflight = self.sum_of_squares_inflight.saturating_sub((self.buckets_inflight[self.buckets_index] as u128).pow(2));
        self.buckets_inflight[self.buckets_index] = inflight as u64;

        self.runner_consumed += consumed as u128;
        self.runner_consumed -= self.buckets_consumed[self.buckets_index] as u128;

        self.percentile_consumed[64 - consumed.leading_zeros() as usize] += 1;
        let old_zeros = self.buckets_consumed[self.buckets_index].leading_zeros() as usize;
        self.percentile_consumed[old_zeros] = self.percentile_consumed[64 - old_zeros].saturating_sub(1);

        let consumed_square = (consumed as u128).pow(2);
        self.sum_of_squares_consumed = self.sum_of_squares_consumed.saturating_add(consumed_square);
        self.sum_of_squares_consumed = self.sum_of_squares_consumed.saturating_sub((self.buckets_consumed[self.buckets_index] as u128).pow(2));
        self.buckets_consumed[self.buckets_index] = consumed as u64;

        self.buckets_index = (1 + self.buckets_index) & self.buckets_mask;
    }

    fn compute_percentile_est(window: u32, pct: f32, ptable: [u32; 65]) -> u128 {
        let limit: u32 = (window as f32 * pct) as u32;
        let mut walker = 0usize;
        let mut sum = 0u32;
        while walker < 62 && sum + ptable[walker] < limit {
            sum += ptable[walker];
            walker += 1;
        }
        1<<walker
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
                (self.runner_inflight * *percent_full_den as u128)
                    .gt(&( (*percent_full_num as u128 * self.capacity as u128) <<  self.buckets_bits) )
            }
            Trigger::AvgFilledAbove(Filled::Exact(exact_full)) => {
                self.runner_inflight.gt(&((*exact_full as u128) << self.buckets_bits as u128))
            }

            Trigger::AvgFilledBelow(Filled::Percentage(percent_full_num, percent_full_den)) => {
                (self.runner_inflight * *percent_full_den as u128)
                    .lt(&( (*percent_full_num as u128 * self.capacity as u128) <<  self.buckets_bits) )
            }
            Trigger::AvgFilledBelow(Filled::Exact(exact_full)) => {
                self.runner_inflight.lt(&((*exact_full as u128) << self.buckets_bits as u128))
            }

            Trigger::StdDevsFilledAbove(std_devs, Filled::Percentage(percent_full_num, percent_full_den)) => {
                let std_deviation = Self::compute_std_dev(self.buckets_bits
                                                           , 1<<self.buckets_bits
                                                           , self.runner_inflight
                                                           , self.sum_of_squares_inflight);
                let std_deviation = (std_deviation * std_devs.value()) as u128;
                let avg = self.runner_inflight >> self.buckets_bits;
                // inflight >  avg + f*std
                ((avg+std_deviation) * *percent_full_den as u128).gt(&(*percent_full_num as u128 * self.capacity as u128 ))
            }
            Trigger::StdDevsFilledAbove(std_devs, Filled::Exact(exact_full)) => {
                let std_deviation = Self::compute_std_dev(self.buckets_bits
                                                          , 1<<self.buckets_bits
                                                          , self.runner_inflight
                                                          , self.sum_of_squares_inflight);
                let std_deviation = (std_deviation * std_devs.value()) as u128;
                let avg = self.runner_inflight >> self.buckets_bits;
                // inflight >  avg + f*std
                (avg+std_deviation).gt(&(*exact_full as u128))
            }

            Trigger::StdDevsFilledBelow(std_devs, Filled::Percentage(percent_full_num, percent_full_den)) => {
                let std_deviation = Self::compute_std_dev(self.buckets_bits
                                                          , 1<<self.buckets_bits
                                                          , self.runner_inflight
                                                          , self.sum_of_squares_inflight);
                let std_deviation = (std_deviation * std_devs.value()) as u128;
                let avg = self.runner_inflight >> self.buckets_bits;
                // inflight >  avg + f*std
                ((avg+std_deviation) * *percent_full_den as u128).lt(&(*percent_full_num as u128 * self.capacity as u128 ))
            }
            Trigger::StdDevsFilledBelow(std_devs, Filled::Exact(exact_full)) => {
                let std_deviation = Self::compute_std_dev(self.buckets_bits
                                                          , 1<<self.buckets_bits
                                                          , self.runner_inflight
                                                          , self.sum_of_squares_inflight);
                let std_deviation = (std_deviation * std_devs.value()) as u128;
                let avg = self.runner_inflight >> self.buckets_bits;
                // inflight >  avg + f*std
                (avg+std_deviation).lt(&(*exact_full as u128))
            }

            Trigger::PercentileFilledAbove(percentile, Filled::Percentage(percent_full_num, percent_full_den)) => {
                let in_flight = Self::compute_percentile_est(1<<self.buckets_bits as u32, percentile.value(), self.percentile_inflight);
                (in_flight * *percent_full_den as u128).gt(&(*percent_full_num as u128 * self.capacity as u128 ))
            }

            Trigger::PercentileFilledAbove(percentile, Filled::Exact(exact_full)) => {
                let in_flight = Self::compute_percentile_est(1<<self.buckets_bits as u32, percentile.value(), self.percentile_inflight);
                in_flight.gt(&(*exact_full as u128))
            }

            Trigger::PercentileFilledBelow(percentile, Filled::Percentage(percent_full_num, percent_full_den)) => {
                let in_flight = Self::compute_percentile_est(1<<self.buckets_bits as u32, percentile.value(), self.percentile_inflight);
                (in_flight * *percent_full_den as u128).lt(&(*percent_full_num as u128 * self.capacity as u128 ))
            }

            Trigger::PercentileFilledBelow(percentile, Filled::Exact(exact_full)) => {
                let in_flight = Self::compute_percentile_est(1<<self.buckets_bits as u32, percentile.value(), self.percentile_inflight);
                in_flight.lt(&(*exact_full as u128))
            }

            /////////////////////////////////////////////////////////////////////////////////////
            /////////////////////////////////////////////////////////////////////////////////////
            // Rule: all rate values are in message per second so they can remain fixed while other values change

            Trigger::AvgRateBelow(rate) => {
                let measured_rate_per_window = self.runner_consumed;
                let window_in_ms = config::TELEMETRY_PRODUCTION_RATE_MS<<self.buckets_bits;
                (measured_rate_per_window * 1000 * rate.to_rational_per_second().1 as u128)
                    .lt(&(window_in_ms as u128 * rate.to_rational_per_second().0 as u128 ))
            },
            Trigger::AvgRateAbove(rate) => {
                let measured_rate_per_window = self.runner_consumed;
                let window_in_ms = config::TELEMETRY_PRODUCTION_RATE_MS<<self.buckets_bits;
                (measured_rate_per_window * 1000 * rate.to_rational_per_second().1 as u128)
                    .gt(&(window_in_ms as u128 * rate.to_rational_per_second().0 as u128 ))
            },
            Trigger::StdDevRateBelow(std_devs, rate) => {
                let std_deviation = Self::compute_std_dev(self.buckets_bits
                                                               , 1<<self.buckets_bits
                                                               , self.runner_consumed
                                                               , self.sum_of_squares_consumed);
                let std_deviation = ((std_deviation as f32 * std_devs.value()) * (( 1<<self.buckets_bits) as f32)) as u128;
                let measured_rate_per_window = self.runner_consumed + std_deviation;
                let window_in_ms = config::TELEMETRY_PRODUCTION_RATE_MS<<self.buckets_bits;
                (measured_rate_per_window * 1000 * rate.to_rational_per_second().1 as u128)
                    .lt(&(window_in_ms as u128 * rate.to_rational_per_second().0 as u128 ))
            }
            Trigger::StdDevRateAbove(std_devs, rate) => {
                let std_deviation = Self::compute_std_dev(self.buckets_bits
                                                               , 1<<self.buckets_bits
                                                               , self.runner_consumed
                                                               , self.sum_of_squares_consumed);
                let std_deviation = ((std_deviation as f32 * std_devs.value()) * (( 1<<self.buckets_bits) as f32)) as u128;
                let measured_rate_per_window = self.runner_consumed + std_deviation;
                let window_in_ms = config::TELEMETRY_PRODUCTION_RATE_MS<<self.buckets_bits;
                (measured_rate_per_window * 1000 * rate.to_rational_per_second().1 as u128)
                    .gt(&(window_in_ms as u128 * rate.to_rational_per_second().0 as u128 ))
            }
            Trigger::PercentileRateAbove(percentile, rate) => {
                let measured_rate = Self::compute_percentile_est(1<<self.buckets_bits as u32, percentile.value(), self.percentile_consumed);
                let window_in_ms = (config::TELEMETRY_PRODUCTION_RATE_MS<<self.buckets_bits) as u128;
                (measured_rate * 1000  * rate.to_rational_per_second().1 as u128).gt(&(window_in_ms * rate.to_rational_per_second().0 as u128))
            }
            Trigger::PercentileRateBelow(percentile, rate) => {
                let measured_rate = Self::compute_percentile_est(1<<self.buckets_bits as u32, percentile.value(), self.percentile_consumed);
                let window_in_ms = (config::TELEMETRY_PRODUCTION_RATE_MS<<self.buckets_bits) as u128;
                (measured_rate * 1000* rate.to_rational_per_second().1 as u128).lt(&(window_in_ms * rate.to_rational_per_second().0 as u128))
            }

            ////////////////////////////////////////
            ////////////////////////////////////////

            Trigger::AvgLatencyAbove(duration) => {
                let avg_inflight = self.runner_inflight >> self.buckets_bits;
                let window_in_micros = 1000u128*(config::TELEMETRY_PRODUCTION_RATE_MS as u128)<<self.buckets_bits;
                let consumed_per_window = self.runner_consumed; //both sides * by this to avoid the division
                (avg_inflight * window_in_micros).gt(&(duration.as_micros()*consumed_per_window) )
            }
            Trigger::AvgLatencyBelow(duration) => {
                let avg_inflight = self.runner_inflight >> self.buckets_bits;
                let window_in_micros = 1000u128*(config::TELEMETRY_PRODUCTION_RATE_MS as u128)<<self.buckets_bits;
                let consumed_per_window = self.runner_consumed; //both sides * by this to avoid the division
                (avg_inflight * window_in_micros).lt(&(duration.as_micros()*consumed_per_window) )
            }

            Trigger::StdDevLatencyAbove(std_devs,duration) => {
                let avg_inflight = self.runner_inflight >> self.buckets_bits;
                let window_in_micros = 1000u128*(config::TELEMETRY_PRODUCTION_RATE_MS as u128)<<self.buckets_bits;
                let consumed_per_window = self.runner_consumed; //both sides * by this to avoid the division

                let consumed_std_deviation = Self::compute_std_dev(self.buckets_bits
                                                                        , 1<<self.buckets_bits
                                                                        , self.runner_consumed
                                                                        , self.sum_of_squares_consumed);

                let adj_consumed_per_window = consumed_per_window + ((consumed_std_deviation * std_devs.value()) as u128) << self.buckets_bits;

                let inflight_std_deviation = Self::compute_std_dev(self.buckets_bits
                                                                        , 1<<self.buckets_bits
                                                                        , self.runner_inflight
                                                                        , self.sum_of_squares_inflight);
                let adj_inflight = avg_inflight + (inflight_std_deviation * std_devs.value()) as u128;

                (adj_inflight * window_in_micros).gt(&(duration.as_micros()*adj_consumed_per_window) )
            }
            Trigger::StdDevLatencyBelow(std_devs,duration) => {
                let avg_inflight = self.runner_inflight >> self.buckets_bits;
                let window_in_micros = 1000u128*(config::TELEMETRY_PRODUCTION_RATE_MS as u128)<<self.buckets_bits;
                let consumed_per_window = self.runner_consumed; //both sides * by this to avoid the division

                let consumed_std_deviation = Self::compute_std_dev(self.buckets_bits
                                                                   , 1<<self.buckets_bits
                                                                   , self.runner_consumed
                                                                   , self.sum_of_squares_consumed);

                let adj_consumed_per_window = consumed_per_window + ((consumed_std_deviation * std_devs.value()) as u128) << self.buckets_bits;

                let inflight_std_deviation = Self::compute_std_dev(self.buckets_bits
                                                                   , 1<<self.buckets_bits
                                                                   , self.runner_inflight
                                                                   , self.sum_of_squares_inflight);
                let adj_inflight = avg_inflight + (inflight_std_deviation * std_devs.value()) as u128;

                (adj_inflight * window_in_micros).lt(&(duration.as_micros()*adj_consumed_per_window) )
            }

            Trigger::LatencyPercentileAbove(percentile, duration) => {
                let consumed_rate = Self::compute_percentile_est(1<<self.buckets_bits as u32, percentile.value(), self.percentile_consumed);
                let in_flight = Self::compute_percentile_est(1<<self.buckets_bits as u32, percentile.value(), self.percentile_inflight);

                let unit_in_micros = 1000u128*(config::TELEMETRY_PRODUCTION_RATE_MS as u128);
                (consumed_rate * unit_in_micros).gt(&(duration.as_micros()* in_flight) )

            }
            Trigger::LatencyPercentileBelow(percentile, duration) => {
                let consumed_rate = Self::compute_percentile_est(1<<self.buckets_bits as u32, percentile.value(), self.percentile_consumed);
                let in_flight = Self::compute_percentile_est(1<<self.buckets_bits as u32, percentile.value(), self.percentile_inflight);

                let unit_in_micros = 1000u128*(config::TELEMETRY_PRODUCTION_RATE_MS as u128);
                (consumed_rate * unit_in_micros).lt(&(duration.as_micros()*in_flight) )
            }

        }
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

    #[test]
    fn test_avg_filled_above_trigger() {

        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 4;
        let mut computer = ChannelStatsComputer::new(&Arc::new(cmd));

        let c = 1<< computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame((computer.capacity as f32 * 0.81f32) as u64
                                           , 100);
        }

        // Define a trigger where the average filled above is 80%
        let trigger = Trigger::AvgFilledAbove(Filled::p80());

        // Test the trigger
        assert!(computer.triggered(&trigger), "Trigger should fire when the average filled is above 80%");
    }

    #[test]
    fn test_avg_filled_below_trigger() {
        // Similar setup as above but with different 'runner_inflight' and 'capacity'
        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 4;
        let mut computer = ChannelStatsComputer::new(&Arc::new(cmd));
        let c = 1<< computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame(65, 100);
        }

        // Define a trigger where the average filled below is 60%
        let trigger = Trigger::AvgFilledBelow(Filled::p60());

        // Test the trigger
        assert!(computer.triggered(&trigger), "Trigger should fire when the average filled is below 60%");
    }


    #[test]
    fn test_avg_filled_above_fixed_trigger() {

        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 4;
        let mut computer = ChannelStatsComputer::new(&Arc::new(cmd));

        let c = 1<< computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame((computer.capacity as f32 * 0.81f32) as u64
                                           , 100);
        }

        // Define a trigger where the average filled above is 80%
        let trigger = Trigger::AvgFilledAbove(Filled::Exact(16));

        // Test the trigger
        assert!(computer.triggered(&trigger), "Trigger should fire when the average filled is above 80%");
    }

    #[test]
    fn test_avg_filled_below_fixed_trigger() {
        // Similar setup as above but with different 'runner_inflight' and 'capacity'
        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 4;
        let mut computer = ChannelStatsComputer::new(&Arc::new(cmd));
        let c = 1<< computer.buckets_bits;
        for _ in 0..c {
            computer.accumulate_data_frame(65, 100);
        }

        // Define a trigger where the average filled below is 60%
        let trigger = Trigger::AvgFilledBelow(Filled::Exact(67));

        // Test the trigger
        assert!(computer.triggered(&trigger), "Trigger should fire when the average filled is below 60%");
    }

    #[test]
    fn test_std_devs_filled_above_trigger() {
        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 4;
        let mut computer = ChannelStatsComputer::new(&Arc::new(cmd));

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
        let trigger = Trigger::StdDevsFilledAbove(StdDev::one(), Filled::p80());

        // Test the trigger
        assert!(computer.triggered(&trigger), "Trigger should fire when standard deviation from the average filled is above the threshold");
    }

    #[test]
    fn test_std_devs_filled_below_trigger() {
        let mut cmd = ChannelMetaData::default();
        cmd.capacity = 256;
        cmd.window_bucket_in_bits = 4;
        let mut computer = ChannelStatsComputer::new(&Arc::new(cmd));

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
        let trigger = Trigger::StdDevsFilledBelow(StdDev::one(), Filled::p70());

        // Test the trigger
        assert!(computer.triggered(&trigger), "Trigger should fire when standard deviation from the average filled is above the threshold");
    }

}
