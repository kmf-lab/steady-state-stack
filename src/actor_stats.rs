use std::cmp::Ordering;
use std::process::exit;
use std::sync::Arc;
use std::time::Duration;
#[allow(unused_imports)]
use log::*;
use num_traits::Zero;
use crate::{config, Filled, Percentile, Rate, StdDev, Trigger};
use crate::monitor::ChannelMetaData;

pub struct ActorStatsComputer {

    pub(crate) red_trigger: Vec<Trigger>, //if used base is green
    pub(crate) yellow_trigger: Vec<Trigger>, //if used base is green

    pub(crate) time_label: String,


}

