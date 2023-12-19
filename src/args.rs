use structopt_derive::StructOpt;

#[derive(StructOpt, Debug, PartialEq)]
pub struct Opt {
    #[structopt(
    short = "n",
    long = "name",
    default_value = "unknown"
    )]
    pub peers_string: String,

    #[structopt(short = "a", long = "age", default_value = "0", validator = |p|
    match p.parse::<u8>() {
    Ok(i) if i <= 120 => Ok(()),
    _ => Err(String::from("Age must be 120 or less.")),
    }
    )]
    pub my_idx: usize,

    #[structopt(long = "logging_level", default_value = "info")]
    pub logging_level: String

}
