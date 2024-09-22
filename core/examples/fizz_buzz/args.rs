use structopt_derive::StructOpt;
use steady_state::SystemdCommand;

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
    pub(crate) fn to_cli_string(&self, app: &str) -> String {
        format!("{} --loglevel={}" //NOTE: add other args here as needed
                , app
                , self.loglevel
               )
    }
    pub(crate) fn systemd_action(&self) -> SystemdCommand {
        if self.systemd_install {
            SystemdCommand::Install
        } else if self.systemd_uninstall {
            SystemdCommand::Uninstall
        } else {
            SystemdCommand::None
        }
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