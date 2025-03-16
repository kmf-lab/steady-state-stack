use crate::LogLevel;
use clap::*;

#[derive(Parser, Debug, PartialEq, Clone)]
pub struct Args {

    #[arg(short = 'l', long = "loglevel", default_value = "info")]
    pub(crate) loglevel: LogLevel,

    #[arg(short = 'g', long = "gen-rate", default_value = "1000")] //1000 is one millisecond
    pub(crate) gen_rate_micros: u64,

    #[arg(short = 'd', long = "duration", value_parser  = run_duration_validator
                                     , default_value = "300")]
    pub(crate) duration: u64,

}
fn run_duration_validator(val: &str) -> Result<u64, String> {
    match val.parse::<u64>() {
        Ok(i) if i <= 300 => Ok(i),
        Ok(_) => Err("Duration must be 300 or less.".to_string()),
        Err(_) => Err("Duration must be a valid u64 number.".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use crate::LogLevel;
    use log::*;
}