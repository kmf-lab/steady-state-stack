
use steady_state::LogLevel;
use steady_state::SystemdCommand;
use clap::*;
#[derive(Parser, Debug, PartialEq, Clone)]
pub struct Args {

    #[arg(short = 'l', long = "loglevel"
                , default_value = "info")]
    pub(crate) loglevel: LogLevel,

    #[arg(short = 'g', long = "gen-rate"
    , default_value = "5000")] //half a millisecond 500 5000 is 5ms
    pub(crate) gen_rate_micros: u64,

    #[arg(short = 'd', long = "duration", value_parser = run_duration_validator
    , default_value = "120")] //use zero for run forever
    pub(crate) duration: u64,

    #[arg(short = 'i', long = "install")]
    pub(crate) systemd_install: bool,

    #[arg(short = 'u', long = "uninstall")]
    pub(crate) systemd_uninstall: bool,

}

fn run_duration_validator(val: &str) -> Result<u64, String> {
    match val.parse::<u64>() {
        Ok(i) if i <= 300 => Ok(i),
        Ok(_) => Err("Duration must be 300 or less.".to_string()),
        Err(_) => Err("Duration must be a valid u64 number.".to_string()),
    }
}



//TODO: in the future these will be derived.
impl Args {

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
use clap::Parser;
use steady_state::LogLevel;
use steady_state::SystemdCommand;
use crate::args::{Args, run_duration_validator};




    #[test]
    fn test_run_duration_validator() {
        assert!(run_duration_validator("100").is_ok());
        assert!(run_duration_validator("240").is_ok());
        assert!(run_duration_validator("241").is_err());
        assert!(run_duration_validator("invalid").is_err());
    }

    #[test]
    fn test_systemd_command_mapping() {
        let args = Args {
            loglevel: LogLevel::Info,
            gen_rate_micros: 5000,
            duration: 120,
            systemd_install: true,
            systemd_uninstall: false,
        };
        assert_eq!(args.systemd_action(), SystemdCommand::Install);

        let args = Args {
            loglevel: LogLevel::Info,
            gen_rate_micros: 5000,
            duration: 120,
            systemd_install: false,
            systemd_uninstall: true,
        };
        assert_eq!(args.systemd_action(), SystemdCommand::Uninstall);

        let args = Args {
            loglevel: LogLevel::Info,
            gen_rate_micros: 5000,
            duration: 120,
            systemd_install: false,
            systemd_uninstall: false,
        };
        assert_eq!(args.systemd_action(), SystemdCommand::None);
    }


    #[test]
    fn test_invalid_log_level() {
        let result = Args::try_parse_from(&[
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
        let result = Args::try_parse_from(&[
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