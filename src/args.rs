use flexi_logger::LogSpecification;
use structopt_derive::StructOpt;

#[derive(StructOpt, Debug, PartialEq, Clone)]
pub struct Args {
    #[structopt(short = "r", long = "run_seconds"
              , default_value = "20"
              , validator = run_duration_validator)]
    pub run_duration: u64,

    #[structopt(short = "l", long = "logging_level"
                , default_value = "info"
                , possible_values = log_variants()
                , validator = validate_logging_level
                , case_insensitive = true)]
    pub logging_level: String,

    #[structopt(short = "g", long = "gen_rate"
    , default_value = "2000")]
    pub gen_rate_ms: u64,
}

fn run_duration_validator(val: String) -> Result<(), String> {
    match val.parse::<u64>() {
        Ok(i) if i <= 120 => Ok(()),
        _ => Err(String::from("run must be 120 or less.")),
    }
}
fn log_variants() -> &'static [&'static str] {
    &["error", "warn", "info", "debug", "trace"]
}

fn validate_logging_level(level: String) -> Result<(), String> {
    LogSpecification::parse(level)
        .map(|_| ())
        .map_err(|_| String::from("Invalid logging level format."))
}


#[cfg(test)]
mod tests {

    //TODO: show how wwe can test the args round trip.


}