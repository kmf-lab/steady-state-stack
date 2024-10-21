
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

    #[structopt(short = "g", long = "gen-rate"
    , default_value = "5000")] //half a millisecond 500 5000 is 5ms
    pub(crate) gen_rate_micros: u64,

    #[structopt(short = "d", long = "duration", validator = run_duration_validator
    , default_value = "120")] //use zero for run forever
    pub(crate) duration: u64,

    #[structopt(short = "i", long = "install")]
    pub(crate) systemd_install: bool,

    #[structopt(short = "u", long = "uninstall")]
    pub(crate) systemd_uninstall: bool,

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
    let level_lower = level.to_lowercase();
    let valid_levels = log_variants();
    if valid_levels.contains(&level_lower.as_str()) {
        Ok(())
    } else {
        Err(String::from("Invalid logging level format."))
    }
}

//TODO: in the future these will be derived.
impl Args {
    pub(crate) fn to_cli_string(&self, app: &str) -> String {
        format!("{} --duration={} --loglevel={} --gen-rate={}"
                , app
                , self.duration
                , self.loglevel
                , self.gen_rate_micros)
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

#[cfg(test)]
mod tests {
    use log::*;
    use structopt::StructOpt;
    use steady_state::SystemdCommand;
    use crate::args::{Args, log_variants, run_duration_validator, validate_logging_level};

    #[test]
    fn test_args_round_trip() {

        steady_state::util::logger::initialize();

        let orig_args = &Args {
            loglevel: "debug".to_string(),
            gen_rate_micros: 3000000,
            duration: 7,
            systemd_install: false,
            systemd_uninstall: false,
        };
        let to_test = orig_args.to_cli_string("myapp");
        trace!("to_test: {}", to_test);
        let cli_args = Args::from_iter(to_test.split_whitespace());
        assert_eq!(cli_args, *orig_args);
    }

    #[test]
    fn test_log_level_validation() {
        assert!(validate_logging_level("debug".to_string()).is_ok());
        assert!(validate_logging_level("DEBUG".to_string()).is_ok());
        assert!(validate_logging_level("invalid".to_string()).is_err());
    }

    #[test]
    fn test_run_duration_validator() {
        assert!(run_duration_validator("100".to_string()).is_ok());
        assert!(run_duration_validator("240".to_string()).is_ok());
        assert!(run_duration_validator("241".to_string()).is_err());
        assert!(run_duration_validator("invalid".to_string()).is_err());
    }

    #[test]
    fn test_systemd_command_mapping() {
        let args = Args {
            loglevel: "info".to_string(),
            gen_rate_micros: 5000,
            duration: 120,
            systemd_install: true,
            systemd_uninstall: false,
        };
        assert_eq!(args.systemd_action(), SystemdCommand::Install);

        let args = Args {
            loglevel: "info".to_string(),
            gen_rate_micros: 5000,
            duration: 120,
            systemd_install: false,
            systemd_uninstall: true,
        };
        assert_eq!(args.systemd_action(), SystemdCommand::Uninstall);

        let args = Args {
            loglevel: "info".to_string(),
            gen_rate_micros: 5000,
            duration: 120,
            systemd_install: false,
            systemd_uninstall: false,
        };
        assert_eq!(args.systemd_action(), SystemdCommand::None);
    }

    #[test]
    fn test_log_variants() {
        let variants = log_variants();
        assert_eq!(variants, ["error", "warn", "info", "debug", "trace"]);
    }

    #[test]
    fn test_invalid_log_level() {
        let result = Args::from_iter_safe(&[
            "myapp",
            "--loglevel",
            "invalid",
            "--gen-rate",
            "5000",
            "--duration",
            "120"
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_duration() {
        let result = Args::from_iter_safe(&[
            "myapp",
            "--loglevel",
            "info",
            "--gen-rate",
            "5000",
            "--duration",
            "241"
        ]);
        assert!(result.is_err());
    }

}