
use clap::*;
use steady_state::*;

#[derive(Parser, Debug, PartialEq, Clone)]
pub struct Args {

    #[arg(short = 'l', long = "loglevel"
                , default_value = "info")]
    pub(crate) loglevel: LogLevel,

    #[arg(short = 'i', long = "install")]
    pub(crate) systemd_install: bool,

    #[arg(short = 'u', long = "uninstall")]
    pub(crate) systemd_uninstall: bool,

}





#[cfg(test)]
mod arg_tests {
    use super::*;

    #[test]
    fn test_args_parsing() {
        let args = Args::try_parse_from(&[
            "test_app",
            "--loglevel", "info",
            "--install",
        ]).unwrap();
        assert_eq!(args.loglevel, LogLevel::Info);
        assert!(args.systemd_install);
        assert!(!args.systemd_uninstall);

        let args = Args::try_parse_from(&[
            "test_app",
            "-l", "debug",
            "-u",
        ]).unwrap();
        assert_eq!(args.loglevel, LogLevel::Debug);
        assert!(!args.systemd_install);
        assert!(args.systemd_uninstall);
    }



}
