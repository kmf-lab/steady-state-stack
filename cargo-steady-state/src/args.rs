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
    let level_lower = level.to_lowercase();
    let valid_levels = log_variants();
    if valid_levels.contains(&level_lower.as_str()) {
        Ok(())
    } else {
        Err(String::from("Invalid logging level format."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use structopt::StructOpt;

    #[test]
    fn test_default_values() {
        let args = Args::from_iter_safe(&[""]).unwrap();
        assert_eq!(args.loglevel, "info");
        assert_eq!(args.dotfile, "graph.dot");
        assert_eq!(args.name, "unnamed");
    }

    #[test]
    fn test_custom_values() {
        let args = Args::from_iter_safe(&[
            "",
            "-l", "debug",
            "-d", "custom.dot",
            "-n", "custom_name"
        ]).unwrap();
        assert_eq!(args.loglevel, "debug");
        assert_eq!(args.dotfile, "custom.dot");
        assert_eq!(args.name, "custom_name");
    }

    #[test]
    fn test_invalid_logging_level() {
        let result = Args::from_iter_safe(&["", "-l", "invalid_level"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_logging_level_variants() {
        for &level in log_variants() {
            assert!(validate_logging_level(level.to_string()).is_ok());
        }
    }

    #[test]
    fn test_invalid_logging_level_validation() {
        let invalid_level = "invalid_level".to_string();
        let result = validate_logging_level(invalid_level);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Invalid logging level format.");
    }
}


