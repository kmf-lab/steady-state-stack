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
    let level_lower = level.to_lowercase();
    let valid_levels = log_variants();
    if valid_levels.contains(&level_lower.as_str()) {
        Ok(())
    } else {
        Err(String::from("Invalid logging level format."))
    }
}

#[cfg(test)]
mod arg_tests {
    use log::trace;
    use super::*;
    use structopt::StructOpt;

    #[test]
    fn test_args_parsing() {
        let args = Args::from_iter_safe(&[
            "test_app",
            "--loglevel", "info",
            "--install",
        ]).unwrap();
        assert_eq!(args.loglevel, "info");
        assert!(args.systemd_install);
        assert!(!args.systemd_uninstall);

        let args = Args::from_iter_safe(&[
            "test_app",
            "-l", "debug",
            "-u",
        ]).unwrap();
        assert_eq!(args.loglevel, "debug");
        assert!(!args.systemd_install);
        assert!(args.systemd_uninstall);
    }

    #[test]
    fn test_to_cli_string() {
        let args = Args {
            loglevel: String::from("info"),
            systemd_install: false,
            systemd_uninstall: false,
        };
        let cli_string = args._to_cli_string("test_app");
        assert_eq!(cli_string, "test_app --loglevel=info");
    }

    #[test]
    fn test_log_variants() {
        let variants = log_variants();
        assert_eq!(variants, &["error", "warn", "info", "debug", "trace"]);
    }

    #[test]
    fn test_validate_logging_level() {
        let valid_levels = vec!["error", "warn", "info", "debug", "trace"];
        for level in valid_levels {
            assert!(validate_logging_level(level.to_string()).is_ok());
        }

        let invalid_levels = vec!["invalid", "verbose", "none"];
        for level in invalid_levels {
            trace!("level: {}", level);
            assert!(validate_logging_level(level.to_string()).is_err());
        }
    }

    #[test]
    fn test_case_insensitive_logging_level() {
        let valid_levels = vec!["ERROR", "Warn", "Info", "DeBuG", "TRACE"];
        for level in valid_levels {
            assert!(validate_logging_level(level.to_string()).is_ok());
        }
    }
}
