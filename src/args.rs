use flexi_logger::LogSpecification;
use structopt_derive::StructOpt;

#[derive(StructOpt, Debug, PartialEq, Clone)]
pub struct Args {

    #[structopt(short = "l", long = "loglevel"
                , default_value = "info"
                , possible_values = log_variants()
                , validator = validate_logging_level
                , case_insensitive = true)]
    pub(crate) loglevel: String,

    #[structopt(short = "g", long = "gen-rate"
    , default_value = "500")] //half a millisecond
    pub(crate) gen_rate_micros: u64,

    #[structopt(short = "d", long = "duration", validator = run_duration_validator
    , default_value = "120")]
    pub(crate) duration: u64,

}

fn run_duration_validator(val: String) -> Result<(), String> {
    match val.parse::<u64>() {
        Ok(i) if i <= 240 => Ok(()),
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

//TODO: need a better way to abstract this for systemd.
pub fn to_cli_string(app:&str, arg: &Args) -> String {
    format!("{} --duration={} --loglevel={} --gen-rate={}"
            , app
            , arg.duration
            , arg.loglevel
            , arg.gen_rate_micros)
}

#[cfg(test)]

#[cfg(test)]
mod tests {
    use log::*;
    use structopt::StructOpt;
    use crate::args::Args;


    #[test]
    fn test_args_round_trip() {
        use crate::args::to_cli_string;
        crate::steady::tests::initialize_logger();

        let orig_args = &Args {
            loglevel: "debug".to_string(),
            gen_rate_micros: 3000000,
            duration: 7
        };
        let to_test = to_cli_string("myapp", orig_args);
        trace!("to_test: {}", to_test);
        let cli_args = Args::from_iter(to_test.split_whitespace());
        assert_eq!(cli_args, *orig_args);
    }

}