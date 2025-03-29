//! # cargo-steady-state - A tool to generate Rust code from a DOT file
//! Dot files can be written by hand or with the help of LLMs. This generates a new empty project.

mod extract_details;
mod args;
mod templates;

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use askama::Template;
use dot_parser::{ast, canonical};
use flexi_logger::{Logger, LogSpecBuilder};
#[allow(unused_imports)]
use log::*;
use clap::*;
use crate::args::Args;
use crate::templates::*;

#[derive(Default)]
struct ProjectModel {
    pub(crate) name: String,
    pub(crate) actors: Vec<Actor>,
    pub(crate) channels: Vec<Vec<Channel>>,
}

//TODO: code gen organize by flow. take actors and create them from inputs down to outputs???

fn main() {
    let opt = Args::parse();

    if let Ok(s) = LevelFilter::from_str(&opt.loglevel) {
        let mut builder = LogSpecBuilder::new();
        builder.default(s); // Set the default level
        let log_spec = builder.build();

        Logger::with(log_spec)
            .format(flexi_logger::colored_with_thread)
            .start().expect("Logger did not start");
    } else {
        eprint!("Warning: Logger initialization failed with bad level: {:?}. There will be no logging.", &opt.loglevel);
    }
    process_dot_file(&opt.dotfile, &opt.name);
}

fn process_dot_file(dotfile: &str, name: &str) {
    match ast::Graph::from_file(dotfile) {
        Ok(ast) => {
            //Dot is clean now try to build the folders
            ///////////
            ///////////
            // Create the project directory
            let mut working_path = PathBuf::from(name);
            if let Err(e) = fs::create_dir_all(&working_path) {
                error!("Failed to create project directory: {}", e);
                return;
            }
            let base_folder_path = working_path.clone();

            // Construct the source folder path
            working_path.push("src");
            if let Err(e) = fs::create_dir_all(&working_path) {
                error!("Failed to create source directory: {}", e);
                return;
            }
            // Save the source folder path for later use
            let source_folder_path = working_path.clone();

            // Construct the actor folder path
            working_path.push("actor");
            if let Err(e) = fs::create_dir_all(&working_path) {
                error!("Failed to create actor directory: {}", e);
                return;
            }
            // Save the actor folder path for later use
            let actor_folder_path = working_path.clone();

            trace!("Project folder created at: {}", base_folder_path.display());
            trace!("Source folder created at: {}", source_folder_path.display());
            trace!("Actor folder created at: {}", actor_folder_path.display());

            ///////////
            ///////////
            //folders done now extract the project details
            ///////////
            match extract_details::extract_project_model(name, canonical::Graph::from(ast)) {
                Ok(project_model) => {
                    trace!("Project model extracted");
                    //last write out the project files
                    if let Err(e) = write_project_files(project_model, &base_folder_path, &source_folder_path, &actor_folder_path) {
                        error!("Partial project files written: {}", e);
                    }
                    println!("New project written to: {}", base_folder_path.display());
                    println!("Check it (expect TODO and unused warnings): cd {} && cargo build", base_folder_path.display());
                }
                Err(e) => {
                     error!("Failed to extract project model: {}", e);
                }
            }
        }
        Err(e) => {
            error!("Failed to parse dot file: {}", e);
        }
    }

}

fn write_project_files(pm: ProjectModel
                       , folder_base: &Path
                       , folder_src: &Path
                       , folder_actor: &Path) -> Result<(), Box<dyn Error>> {

   let cargo = folder_base.join("Cargo.toml");
   fs::write(cargo, templates::CargoTemplate { name: &pm.name }.render()?)?;

    let docker = folder_base.join("Dockerfile");
    fs::write(docker, templates::DockerFileTemplate { name: &pm.name }.render()?)?;

   let gitignore = folder_base.join(".gitignore");
   fs::write(gitignore, templates::GitIgnoreTemplate {}.render()?)?;

   let args_rs = folder_src.join("args.rs");
   fs::write(args_rs, templates::ArgsTemplate {}.render()?)?;

   let main_rs = folder_src.join("main.rs");
   let mut actor_mods:Vec<String> =  pm.actors.iter().map(|f| f.mod_name.clone()).collect();
   actor_mods.sort();
   actor_mods.dedup();

   fs::write(main_rs, templates::MainTemplate {
       note_for_the_user: format!("//TO{}: ","DO"), //do not change, this is not for you
       project_name: pm.name.clone(),        
        actor_mods,
        channels: &pm.channels,
        actors: &pm.actors,
   }.render()?)?;

   for actor in pm.actors {
       let actor_file_rs = folder_actor.join(Path::new(&actor.mod_name)).with_extension("rs");

       //need list of unique message types, do we dedup here??
       let mut my_struct_use:Vec<String> = actor.rx_channels
            .iter()
            .filter(|f| !f[0].from_mod.eq(&f[0].to_mod))//if to and from same place then its not use it will be define later
            .map(|f| format!("use crate::actor::{}::{}",if f[0].from_mod.is_empty() { "UNKNOWN" } else {&f[0].from_mod}, f[0].message_type))
            .collect();

       //NOTE: if we use a struct of the same name we assume it is the same and
       //      do not define it again. if its bundled only define it for the first
       let mut my_struct_def:Vec<String> = actor.tx_channels
           .iter()
           .filter(|f| {
               if !f[0].bundle_struct_mod.is_empty() && !actor.mod_name.eq(&f[0].bundle_struct_mod) &&
                  !f[0].is_unbundled {
                   my_struct_use.push(format!("use crate::actor::{}::{}",
                                              &f[0].bundle_struct_mod,
                                              f[0].message_type));
                   false
               } else {
                   true
               }
              } )
           .filter(|f| actor.rx_channels.iter().all(|g| {
                       //if we have no rx on this actor returns true, nothing to check
                       //if ALL the actor rx to not equal this actor tx type
                       !g[0].message_type.eq(&f[0].message_type)
                       //unless channel is coming from the same place, ie a loopback
                          || g[0].from_mod.eq(&f[0].from_mod)
                    }
           ))
           .map(|f| format!("{}pub(crate) struct {}", derive_block(f[0].copy), f[0].message_type))
           .collect();
       ///////////////////////////
       my_struct_use.sort();
       my_struct_use.dedup();
       my_struct_def.sort();
       my_struct_def.dedup();


       let full_driver_block = build_driver_block(&actor);
     
       //build monitor lists
       let rx_monitor_defs = monitor_defs("rx",&actor.rx_channels);
       let tx_monitor_defs = monitor_defs("tx",&actor.tx_channels);

       fs::write(actor_file_rs, templates::ActorTemplate {
               is_on_graph_edge: actor.is_on_graph_edge(),
               note_for_the_user: format!("//TO{}: ","DO"), //do not change, this is not for you
               //this is for me as an actor so the name does not require suffix
               display_name: actor.display_name,
               has_bundles: actor.rx_channels.iter().any(|f| f.len()>1) || actor.tx_channels.iter().any(|f| f.len()>1),
               rx_channels: actor.rx_channels,
               tx_channels: actor.tx_channels,
               rx_monitor_defs,
               tx_monitor_defs,
               full_driver_block,
               message_types_to_use: my_struct_use,
               message_types_to_define: my_struct_def,
       }.render()?)?;
   }
   Ok(())
}


fn build_driver_block(actor: &Actor) -> String {

    let mut at_least_every: Option<String> = None;
    let mut andy_drivers: Vec<String> = Vec::new();

    actor.driver.iter().for_each(|f| {
        match f {
            ActorDriver::AtLeastEvery(d) => {
                at_least_every = Some(format!("cmd.wait_periodic(Duration::from_millis({:?}))",d.as_millis()));
            }
            ActorDriver::AtMostEvery(d) => {
                andy_drivers.push(format!("cmd.wait_periodic(Duration::from_millis({:?}))",d.as_millis()));
            }
            ActorDriver::EventDriven(t) => {
                let mut each: Vec<String> = t.iter().map(|v| {

                    let girth = actor.rx_channels.iter()
                            .find(|f| f[0].name == v[0])
                            .map(|f| f.len()).unwrap_or(1);

                    //validate the name
                    if actor.rx_channels.iter().all(|f| f[0].name != v[0]) {
                        warn!("Failed to find channel: {}, please fix dot file node label for actor: {}", v[0], actor.display_name);
                    }

                    //2 may want the default channels_count or this may be a single
                    //  we ensure girth is 1 to confirm this choice.
                    if v.len()==2 && 1==girth {
                        format!("cmd.wait_avail(&mut {}_rx,{})", v[0], v[1])
                    } else {
                        let channels_count = if girth>1 && v.len()>2 {
                            if let Some(p) = extract_percent(v[2].clone()) {
                                (girth as f32 * p) as usize
                            } else {
                                girth //default
                            }
                        } else {
                            1 //if we got no girth assume 1 by default
                        };
                        format!("cmd.wait_avail_bundle(&mut {}_rx,{},{})", v[0], v[1], channels_count)
                    }
                }).collect();
                andy_drivers.append(&mut each);
            }
            ActorDriver::CapacityDriven(t) => {
                let mut each: Vec<String> = t.iter().map(|v| {

                    let girth = actor.tx_channels.iter()
                        .find(|f| f[0].name == v[0])
                        .map(|f| f.len()).unwrap_or(1);
                    
                    //validate the name
                    if actor.tx_channels.iter().all(|f| f[0].name != v[0]) {
                        warn!("Failed to find channel: {}, please fix dot file node label for actor: {}", v[0], actor.display_name);
                    }

                    if v.len() == 2 && 1==girth {
                        format!("cmd.wait_shutdown_or_vacant_units(&mut {}_tx,{})", v[0], v[1])
                    } else {
                        let girth = actor.tx_channels
                            .iter()
                            .find(|f| f[0].name == v[0])
                            .map(|f| f.len());
                        let channels_count = if let Some(g) = girth {
                            if let Some(p) = extract_percent(v[2].clone()) {
                                (g as f32 * p) as usize
                            } else {
                                g //if we can not get pct or not provided so assume 100%
                            }
                        } else {
                            warn!("Failed to find more than one channel in the bundle: {}", v[0]);
                            1 //if we got no girth assume 1 by default
                        };
                        format!("cmd.wait_shutdown_or_vacant_units_bundle(&mut {}_tx,{},{})", v[0], v[1], channels_count)
                    }
                }).collect();
                andy_drivers.append(&mut each);
            }
            ActorDriver::Other(t) => {
                let mut each: Vec<String> = t.iter().map(|name| {
                    format!("cmd.call_async({}())", name) 
                }).collect();
                andy_drivers.append(&mut each);
            }
        }
    });

    let mut full_driver_block = String::new();

    if let Some(t) = at_least_every {
        //this block must be a wrapping select around the others

        if andy_drivers.is_empty() {
            full_driver_block.push_str("    let clean = await_for_all!(");
            full_driver_block.push_str(&t);
            full_driver_block.push_str(");\n");
        } else {
            full_driver_block.push_str("    let clean = await_for_all_or_proceed_upon!(");

            full_driver_block.push_str(&t);
            full_driver_block.push('\n');

            //setup for the join
            andy_drivers.iter().for_each(|t| {
                full_driver_block.push(',');
                full_driver_block.push_str(t);
                full_driver_block.push('\n');
            });
            full_driver_block.push_str("    );\n");
        }
    } else {
        full_driver_block.push_str("    let clean = await_for_all!(");
        andy_drivers.iter().enumerate().for_each(|(i,t)| {
            if i>0 {
                full_driver_block.push_str(",\n");
            }
            full_driver_block.push_str(t);
        });
        full_driver_block.push_str("    );\n");
    }

    full_driver_block
}

fn extract_percent(text: String) -> Option<f32> {
    //if text ends with % we then pars the leading int and divide by 100
    if text.ends_with('%') {
        let text = text.trim_end_matches('%');
        if let Ok(pct) = text.parse::<f32>() {
            Some(pct/100.0)
        } else {
            warn!("Failed to parse percent: {}", text);
            None
        }
    } else {
        // if the text is a decimal less than or equal to 1.0 we return it
        if let Ok(pct) = text.parse::<f32>() {
            if pct <= 1.0 {
                Some(pct)
            } else {
                warn!("Failed to parse percent: {}", text);
                None
            }
        } else {
            warn!("Failed to parse percent: {}", text);
            None
        }
    }


}

const DISABLE:bool = false;

fn monitor_defs(direction: &str, channels: &[Vec<Channel>]) -> Vec<String> {
  let mut result = Vec::new();
  let is_alone = channels.len()==1 && DISABLE;
  channels.iter().for_each(|c| {

      if c.len()>1 {
         //  am I alone if so just write else we must do each one.
         //NOTE: this feature disabled until we get better support for use of
          // generic constants to sum two together to build a new constant array.
         if is_alone {//  tx.def_slice(), does not require & or surrounding []
             result.push(format!("{}_{}.def_slice()",c[0].name,direction));
         } else {
             //NOTE: once we have simple expressions of generic constants to sum arrays this
             //     code can be removed
             c.iter()
                 .enumerate()
                 .for_each(|(i,f)| { //NOTE: each requies an index
                 result.push(format!("{}_{}[{}]",f.name,direction,i));
             });
         }
      } else {
          result.push(format!("{}_{}",c[0].name,direction).to_string());
      }
  });

  result
}

fn derive_block(copy: bool) -> &'static str {
    match copy {
        true =>  "#[derive(Default,Clone,Debug,Eq,PartialEq,Copy)]\n",
        false => "#[derive(Default,Clone,Debug,Eq,PartialEq)]\n",
    }
}


#[cfg(test)]
mod tests {
    use std::{env, fs};
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::str::FromStr;
    use colored::Colorize;
    use flexi_logger::{Logger, LogSpecBuilder};
    use log::{error, info, trace, LevelFilter};
    use crate::process_dot_file;

    fn build_and_parse(test_name: &str, graph_dot: &str, clean: bool, show_logs: bool, live_test: bool) {
        if live_test { //this will require open ports and drive access so we do not always run it

            let level = "info";
            if let Ok(s) = LevelFilter::from_str(&level) {
                let mut builder = LogSpecBuilder::new();
                builder.default(s); // Set the default level
                let log_spec = builder.build();
                //turn on for debug
                if show_logs {
                    let result = Logger::with(log_spec)
                        .format(flexi_logger::colored_with_thread)
                        .start();
                    match result {
                        Ok(_) => {
                            trace!("Logger initialized at level: {:?}", &level);
                        }
                        Err(e) => {
                            //just go one with the tests this happens from time to time due to what we are doing.
                            eprint!("Logger initialization failed: {:?}", e);
                        }
                    }
                }
            } else {
                eprint!("Warning: Logger initialization failed with bad level: {:?}. There will be no logging.", &level);
            }
            //ensure test_run folder exists and we are in the right place
            let current_dir = env::current_dir().expect("Failed to get current directory");
    
            //move to our test_run folder to ensure we do not generate test code on top of our self
            let ok = current_dir.ends_with("test_run")
                || match env::set_current_dir("test_run") {
                Ok(_) => {
                    true
                }
                Err(_) => {
                    let working_path = PathBuf::from("test_run");
                    if let Err(e) = fs::create_dir_all(&working_path) {
                        error!("Failed to create test_run directory: {}", e);
                        panic!("Failed to change directory to test_run: {}", e);
                    }
                    if let Err(e) = env::set_current_dir("test_run") {
                        error!("Failed to change directory to test_run: {}", e);
                        panic!("Failed to change directory to test_run: {}", e);
                    }
                    true
                }
            };
    
            if ok {
          
                    let current_dir = env::current_dir().expect("Failed to get current directory");
                    println!("Current working directory is: {:?}", current_dir);
                    if clean {
                        //NOTE: must delete the unnamed1 folder if it exists
                        if let Err(e) = fs::remove_dir_all(test_name) {
                            error!("Failed to remove test_run/{} directory: {}",test_name, e);
                        } else {
                            info!("Removed test_run/{} directory",test_name);
                        }
                    }
                    // write graph_dot into a temp file
                    let dot_file = format!("{}.dot", test_name);
                    fs::write(&dot_file, graph_dot).expect("Failed to write dot file");
                    process_dot_file(&dot_file, test_name);

                println!("____________________________________________________________-----------------------------------------------------------");
                    do_cargo_cache_install(test_name);
                println!("============================================================-----------------------------------------------------------");
                    #[cfg(not(windows))]
                    do_cargo_build_of_generated_code(test_name);
                println!("////////////////////////////////////////////////////////////-----------------------------------------------------------");
                    #[cfg(not(windows))]
                    do_cargo_test_of_generated_code(test_name);

                    let _ignore = fs::remove_file(&dot_file);

            }
        }
}

fn do_cargo_cache_install(test_name: &str) {
    let build_me = PathBuf::from(test_name);
    let build_me_absolute = env::current_dir().unwrap().join(build_me).canonicalize().unwrap();

    // Check if sccache is already installed and executable
    let sccache_check = Command::new("sccache")
        .arg("--version")
        .stdout(Stdio::null()) // Suppress stdout for the version check
        .stderr(Stdio::null()) // Suppress stderr for the version check
        .status();

    match sccache_check {
        Ok(status) if status.success() => {
            // sccache is already installed and returned a successful version check
            println!("sccache is already installed, skipping installation.");
        }
        _ => {
            // sccache is not installed or failed the version check, proceed with installation
            println!("sccache not found or not working, installing...");
            let mut output_child = Command::new("cargo")
                .arg("install")
                .arg("sccache")
                .current_dir(build_me_absolute.clone())
                .stdout(Stdio::inherit()) // Print stdout directly to terminal
                .stderr(Stdio::inherit()) // Print stderr directly to terminal
                .spawn()
                .expect("failed to execute process");

            let output = output_child.wait().expect("failed to wait on child");
            assert!(output.success(), "Failed to install sccache");
        }
    }
}
    fn do_cargo_build_of_generated_code(test_name: &str) {
        // Construct the absolute path to the directory containing Cargo.toml
        let build_me = PathBuf::from(test_name);
        let build_me_absolute = env::current_dir().unwrap().join(build_me).canonicalize().unwrap();

        // Choose the cargo command based on the platform //TODO: not yet working.
        let cargo_cmd = if cfg!(windows) { "check" } else { "build" };

        // Execute the cargo command and capture its output
        let output = Command::new("cargo")
            .arg(cargo_cmd) // Use "check" on Windows, "build" elsewhere
            .arg("--verbose") // Keep verbose output for detailed information
            .arg("--manifest-path")
            .arg(build_me_absolute.join("Cargo.toml").to_str().unwrap()) // Path to Cargo.toml
            .current_dir(build_me_absolute.clone()) // Set the working directory
            .env("RUSTC_WRAPPER", "sccache") // Use sccache for compiler caching (still works with check)
            .stdout(Stdio::piped()) // Capture stdout instead of inheriting it
            .stderr(Stdio::piped()) // Capture stderr instead of inheriting it
            .spawn()
            .expect("failed to execute process")
            .wait_with_output()
            .expect("failed to wait on child");

        // Print output for debugging regardless of success
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("{} stdout for {}:\n{}", cargo_cmd, test_name, stdout);
        eprintln!("{} stderr for {}:\n{}", cargo_cmd, test_name, stderr);

        // Check if the command failed and handle accordingly
        if !output.status.success() {
            eprintln!("{}", format!("{} failed for {}:", cargo_cmd, test_name).red());
            panic!("{} failed for {}", cargo_cmd, test_name); // Panic after printing output
        } else {
            println!("{} succeeded for {}", cargo_cmd, test_name);
        }
    }
    
    fn do_cargo_test_of_generated_code(test_name: &str) {
        let build_me = PathBuf::from(test_name);
        let build_me_absolute = env::current_dir().unwrap().join(build_me).canonicalize().unwrap();
        ////
        let mut output_child = Command::new("cargo")
            .arg("test")
            .arg("--manifest-path")
            .arg(build_me_absolute.join("Cargo.toml").to_str().unwrap()) // Ensure this path points to your generated Cargo.toml
            .env("RUSTC_WRAPPER","sccache")
            .current_dir(build_me_absolute.clone())
            .stdout(Stdio::inherit()) // This line ensures that stdout from the command is printed directly to the terminal
            .stderr(Stdio::inherit()) // This line ensures that stderr is also printed directly to the terminal
            .spawn()
            .expect("failed to execute process");
        let output = output_child.wait().expect("failed to wait on child");
           
        assert!(output.success());
    }


    #[test]
    fn test_fizz_buzz_project() {

        let g = r#"
digraph PRODUCT {
    // Documentation node
    __SystemOverview [shape=box, label="This graph represents a FizzBuzz processing system for an educational video.\nActors produce numbers divisible by 3 and 5.\nA processor combines them into FizzBuzz messages.\nA console printer outputs summary statistics every 10 seconds.\nAn ErrorLogger captures any processing errors."];

    // Actors
    DivBy3Producer [label="Produces numbers divisible by 3\nmod::div_by_3_producer\nAtLeastEvery(1ms)"];
    DivBy5Producer [label="Produces numbers divisible by 5\nmod::div_by_5_producer\nAtLeastEvery(1ms)"];
    FizzBuzzProcessor [label="Generates Fizz, Buzz, and FizzBuzz records\nmod::fizz_buzz_processor\nOnEvent(numbers:1)"];
    ConsolePrinter [label="Prints summary counts to console\nmod::console_printer\nOnEvent(print_signal:1)"];
    TimerActor [label="Triggers printing every 10 seconds\nmod::timer_actor\nAtLeastEvery(10sec)"];
    ErrorLogger [label="Logs processing errors\nmod::error_logger\nOnEvent(errors:1)"];

    // Channels
    DivBy3Producer -> FizzBuzzProcessor [label="name::numbers <NumberMessage> >>TakeCopy #1000"];
    DivBy5Producer -> FizzBuzzProcessor [label="name::numbers <NumberMessage> >>TakeCopy #1000"];
    FizzBuzzProcessor -> ConsolePrinter [label="name::fizzbuzz_messages <FizzBuzzMessage> >>Take #10000"];
    TimerActor -> ConsolePrinter [label="name::print_signal <PrintSignal> >>Take #1"];
    FizzBuzzProcessor -> ErrorLogger [label="name::errors <ErrorMessage> >>Take #100"];


    // Union types and descriptions
    __NumberMessage [shape=box, label="NumberMessage is a union type for numbers from DivBy3Producer and DivBy5Producer.\nVariants: DivBy3Number, DivBy5Number."];
    __NumberMessage -> DivBy3Producer [style=dotted];
    __NumberMessage -> DivBy5Producer [style=dotted];
    __NumberMessage -> FizzBuzzProcessor [style=dotted];

    __FizzBuzzMessage [shape=box, label="FizzBuzzMessage contains 'Fizz', 'Buzz', or 'FizzBuzz' records.\nGenerated by FizzBuzzProcessor for ConsolePrinter."];
    __FizzBuzzMessage -> FizzBuzzProcessor [style=dotted];
    __FizzBuzzMessage -> ConsolePrinter [style=dotted];

    __ErrorMessage [shape=box, label="ErrorMessage captures any errors during processing.\nUsed by FizzBuzzProcessor to send to ErrorLogger."];
    __ErrorMessage -> FizzBuzzProcessor [style=dotted];
    __ErrorMessage -> ErrorLogger [style=dotted];
}
        "#;

       build_and_parse("fizz_buzz", g, false, false,!std::env::var("GITHUB_ACTIONS").is_ok());

    }


    #[test]
    fn test_unnamed1_project() {

        let g = r#"
        digraph PRODUCT {
    ImapClient [label="IMAP Client\nConnects to IMAP server\nto fetch emails"];
    EmailFilter [label="Email Filter\nAnalyzes emails to determine spam"];
    SpamAssassin [label="SpamAssassin Integration\nOptionally uses SpamAssassin for detection"];
    EmailMover [label="Email Mover\nMoves spam emails to a designated folder"];
    ConfigLoader [label="Config Loader\nLoads configuration for IMAP and settings"];
    Logger [label="Logger\nLogs system activities for monitoring"];
    __TossOut [label="TossOut\nnothing here"];

    ConfigLoader -> IMAPClient [label="name::cfg <ServerDetails> Imap server details\nemail, password"];
    ImapClient -> EmailFilter [label="name::email <Email> Emails\nRaw email data"];
    EmailFilter -> SpamAssassin [label="name::content <EmailBody> Email content\nFor SpamAssassin analysis"];
    SpamAssassin -> EmailFilter [label="name::verdict <SpamScore> Spam verdict\nSpam or Ham"];
    EmailFilter -> EmailMover [label="name::spam <SpamBody> Spam emails\nIdentified spam messages"];
    EmailMover -> Logger [label="name::email_log <SuccessLog> Move operations\nSuccess or failure logs"];
    ImapClient -> Logger [label="name::con_log <Connections> Connection logs\nSuccess or failure"];
    EmailFilter -> Logger [label="name::filt_log Filtering logs\nProcessed emails and results"];

    edge [color=blue];
    node [style=filled, color=lightgrey];
}
        "#;

     build_and_parse("unnamed1", g, false, false,!std::env::var("GITHUB_ACTIONS").is_ok());

    }

    #[test]
    fn test_pbft_project() {
        let g = r#"
digraph PBFTDemo {
     Client [label="Initiates requests\nmod::client AtLeastEvery(5sec)"];
     Primary [label="Orders requests and initiates consensus\nmod::primary AtLeastEvery(2ms)&&OnEvent(original_request:1||feedback:1)"];

     Client -> Primary [label="name::original_request <TransactionRequest> >>PeekCopy #20"];
     Replica1 [label="Participates in consensus\nmod::replica OnEvent(pbft_message:1)&&AtMostEvery(1ms)"];
     Replica2 [label="Participates in consensus\nmod::replica OnEvent(pbft_message:1)&&AtMostEvery(1ms)"];
     Replica3 [label="Participates in consensus\nmod::replica OnEvent(pbft_message:1)&&AtMostEvery(1ms)"];

     // Simplified PBFT message exchange using a single channel type
     Primary -> Replica1 [label="name::pbft_message <PbftMessage> >>PeekCopy #30"];
     Primary -> Replica2 [label="name::pbft_message <PbftMessage> >>PeekCopy #30"];
     Primary -> Replica3 [label="name::pbft_message <PbftMessage> >>PeekCopy #30"];

     // Feedback channels from Replicas back to the Primary
     Replica1 -> Primary [label="name::feedback <PbftFeedback> >>PeekCopy #10"];
     Replica2 -> Primary [label="name::feedback <PbftFeedback> >>PeekCopy #10"];
     Replica3 -> Primary [label="name::feedback <PbftFeedback> >>PeekCopy #10"];

     // Replica to Replica communication for prepare and commit phases
     Replica1 -> Replica2 [label="name::pbft_message <PbftMessage> >>PeekCopy #30"];
     Replica1 -> Replica3 [label="name::pbft_message <PbftMessage> >>PeekCopy #30"];

     Replica2 -> Replica1 [label="name::pbft_message <PbftMessage> >>PeekCopy #30"];
     Replica2 -> Replica3 [label="name::pbft_message <PbftMessage> >>PeekCopy #30"];

     Replica3 -> Replica1 [label="name::pbft_message <PbftMessage> >>PeekCopy #30"];
     Replica3 -> Replica2 [label="name::pbft_message <PbftMessage> >>PeekCopy #30"];

}

        "#;

       build_and_parse("pbft_demo", g, false, true, !std::env::var("GITHUB_ACTIONS").is_ok());

    }


    #[test]
    fn test_circle_project() {
        let g = r#"
            digraph Circle {

            rankdir=TB;

            Handler1 [label="Does Work\nmod::handler\nAtLeastEvery(10ms) && OnEvent(token:1)"];
            Handler2 [label="Does Work\nmod::handler\nAtLeastEvery(10ms) && OnEvent(token:1)"];
            Handler3 [label="Does Work\nmod::handler\nAtLeastEvery(10ms) && OnEvent(token:1)"];
            __Token [label="Token is a ........"];

            Handler1 -> Handler2 [label="name::token <Token> >>PeekCopy #10"];
            Handler2 -> Handler3 [label="name::token <Token> >>PeekCopy #10"];
            Handler3 -> Handler1 [label="name::token <Token> >>PeekCopy #10"];
            Handler3 -> __Token [label="Circle go burr"];

            }
        "#;

       build_and_parse("circle", g, false, true, !std::env::var("GITHUB_ACTIONS").is_ok());

    }
}

#[cfg(test)]
mod more_tests {
    use super::*;
    use crate::templates::{Actor, ActorDriver, Channel};
    use std::time::Duration;
    use std::cell::RefCell;
    use crate::extract_percent;

    #[test]
    fn test_extract_percent() {
        assert_eq!(extract_percent("50%".to_string()), Some(0.5));
        assert_eq!(extract_percent("100%".to_string()), Some(1.0));
        assert_eq!(extract_percent("0%".to_string()), Some(0.0));
        assert_eq!(extract_percent("75.5%".to_string()), Some(0.755));
        assert_eq!(extract_percent("0.25".to_string()), Some(0.25));
        assert_eq!(extract_percent("invalid".to_string()), None);
        assert_eq!(extract_percent("".to_string()), None);
        assert_eq!(extract_percent("110%".to_string()), Some(1.1));
    }

    #[test]
    fn test_build_driver_block_with_at_least_every() {
        let actor = Actor {
            display_name: "TestActor".to_string(),
            display_suffix: None,
            mod_name: "test_actor".to_string(),
            rx_channels: vec![],
            tx_channels: vec![],
            driver: vec![ActorDriver::AtLeastEvery(Duration::from_secs(5))],
        };

        let result = build_driver_block(&actor);
        let expected = "cmd.wait_periodic(Duration::from_millis(5000))";
        assert!(result.contains(expected));
    }

    #[test]
    fn test_build_driver_block_with_event_driven() {
        let actor = Actor {
            display_name: "TestActor".to_string(),
            display_suffix: None,
            mod_name: "test_actor".to_string(),
            rx_channels: vec![vec![Channel {
                name: "input".to_string(),
                from_mod: "source_mod".to_string(),
                to_mod: "test_actor".to_string(),
                batch_read: 1,
                batch_write: 1,
                message_type: "InputMessage".to_string(),
                peek: false,
                copy: false,
                capacity: 10,
                bundle_index: -1,
                rebundle_index: -1,
                bundle_struct_mod: "".to_string(),
                to_node: "TestActor".to_string(),
                from_node: "SourceActor".to_string(),
                bundle_on_from: RefCell::new(true),
                is_unbundled: true,
            }]],
            tx_channels: vec![],
            driver: vec![ActorDriver::EventDriven(vec![vec!["input".to_string(), "1".to_string()]])],
        };

        let result = build_driver_block(&actor);
        let expected = "cmd.wait_avail(&mut input_rx,1)";
        assert!(result.contains(expected));
    }

    #[test]
    fn test_monitor_defs_single_channel() {
        let channels = vec![vec![Channel {
            name: "test_channel".to_string(),
            from_mod: "from_mod".to_string(),
            to_mod: "to_mod".to_string(),
            batch_read: 1,
            batch_write: 1,
            message_type: "TestMessage".to_string(),
            peek: false,
            copy: false,
            capacity: 10,
            bundle_index: -1,
            rebundle_index: -1,
            bundle_struct_mod: "".to_string(),
            to_node: "ToNode".to_string(),
            from_node: "FromNode".to_string(),
            bundle_on_from: RefCell::new(true),
            is_unbundled: true,
        }]];

        let result = monitor_defs("rx", &channels);
        assert_eq!(result, vec!["test_channel_rx".to_string()]);
    }

    #[test]
    fn test_monitor_defs_bundled_channels() {
        let channels = vec![
            vec![
                Channel {
                    name: "test_channel".to_string(),
                    from_mod: "from_mod".to_string(),
                    to_mod: "to_mod".to_string(),
                    batch_read: 1,
                    batch_write: 1,
                    message_type: "TestMessage".to_string(),
                    peek: false,
                    copy: false,
                    capacity: 10,
                    bundle_index: 0,
                    rebundle_index: -1,
                    bundle_struct_mod: "".to_string(),
                    to_node: "ToNode".to_string(),
                    from_node: "FromNode".to_string(),
                    bundle_on_from: RefCell::new(true),
                    is_unbundled: false,
                },
                Channel {
                    name: "test_channel".to_string(),
                    from_mod: "from_mod".to_string(),
                    to_mod: "to_mod".to_string(),
                    batch_read: 1,
                    batch_write: 1,
                    message_type: "TestMessage".to_string(),
                    peek: false,
                    copy: false,
                    capacity: 10,
                    bundle_index: 1,
                    rebundle_index: -1,
                    bundle_struct_mod: "".to_string(),
                    to_node: "ToNode".to_string(),
                    from_node: "FromNode".to_string(),
                    bundle_on_from: RefCell::new(true),
                    is_unbundled: false,
                },
            ],
        ];

        let result = monitor_defs("tx", &channels);
        // Since DISABLE is false, it should generate individual channel names
        assert_eq!(result, vec!["test_channel_tx[0]".to_string(), "test_channel_tx[1]".to_string()]);
    }

    #[test]
    fn test_derive_block() {
        let copy_true = derive_block(true);
        assert_eq!(copy_true, "#[derive(Default,Clone,Debug,Eq,PartialEq,Copy)]\n");

        let copy_false = derive_block(false);
        assert_eq!(copy_false, "#[derive(Default,Clone,Debug,Eq,PartialEq)]\n");
    }
}



