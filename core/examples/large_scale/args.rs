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
    , default_value = "1000")] //1000 is one millisecond
    pub(crate) gen_rate_micros: u64,

    #[structopt(short = "d", long = "duration", validator = run_duration_validator
    , default_value = "300")]
    pub(crate) duration: u64,

}

fn run_duration_validator(val: String) -> Result<(), String> {
    match val.parse::<u64>() {
        Ok(i) if i <= 300 => Ok(()),
        _ => Err(String::from("run must be 300 or less.")),
    }
}
fn log_variants() -> &'static [&'static str] {
    &["error", "warn", "info", "debug", "trace"]
}

fn validate_logging_level(level: String) -> Result<(), String> {
    let level_lower = level.to_lowercase();
    let valid_levels = log_variants();
    if valid_levels.contains(&level_lower.as_str()) {
        Ok(())
    } else {
        Err(String::from("Invalid logging level format."))
    }
}

impl Args {
    pub fn to_cli_string(&self, app: &str) -> String {
        format!("{} --duration={} --loglevel={} --gen-rate={}"
                , app
                , self.duration
                , self.loglevel
                , self.gen_rate_micros)
    }

    /*
    pub fn to_cli_string(&self, app: &str) -> Result<String, serde_json::Error> {
        let map = serde_json::to_value(self)?;
        let mut parts = vec![app.to_string()];

        for (key, value) in map.as_object().unwrap() {
            // Convert the field name to a command-line flag
            let flag = format!("--{}", key.replace('_', "-"));
            // Append the flag and its value to the command string
            parts.push(format!("{flag}={value}"));
        }

        Ok(parts.join(" "))
    }*/

}


#[cfg(test)]
mod tests {
    use log::*;
    use structopt::StructOpt;
    use crate::args::Args;

    #[test]
    fn test_args_round_trip() {

        steady_state::util::logger::initialize();

        let orig_args = &Args {
            loglevel: "debug".to_string(),
            gen_rate_micros: 3000000,
            duration: 7
        };
        let to_test = orig_args.to_cli_string("myapp");
        trace!("to_test: {}", to_test);
        let cli_args = Args::from_iter(to_test.split_whitespace());
        assert_eq!(cli_args, *orig_args);
    }

}