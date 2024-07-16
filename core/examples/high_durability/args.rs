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

    #[structopt(short = "i", long = "install")]
    pub(crate) systemd_install: bool,

    #[structopt(short = "u", long = "uninstall")]
    pub(crate) systemd_uninstall: bool,

}

impl Args {
    pub(crate) fn _to_cli_string(&self, app: &str) -> String {
        format!("{} --loglevel={}" //NOTE: add other args here as needed
                , app
                , self.loglevel
               )
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