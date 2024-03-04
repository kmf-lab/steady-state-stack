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

    #[structopt(short = "d", long = "dotfile"
                             , default_value = "graph.dot")]
    pub(crate) dotfile: String,

    #[structopt(short = "n", long = "name"
                , default_value = "unnamed")]
    pub(crate) name: String,

    //other global  switches,  --disable-testing --import-actors ??

}

fn log_variants() -> &'static [&'static str] {
    &["error", "warn", "info", "debug", "trace"]
}

fn validate_logging_level(level: String) -> Result<(), String> {
    LogSpecification::parse(level)
        .map(|_| ())
        .map_err(|_| String::from("Invalid logging level format."))
}


