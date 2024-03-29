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
use structopt::StructOpt;
use crate::args::Args;
use crate::templates::*;

#[derive(Default)]
struct ProjectModel {
    pub(crate) name: String,
    pub(crate) actors: Vec<Actor>,
    pub(crate) channels: Vec<Vec<Channel>>,
}

fn main() {
    let opt = Args::from_args();


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

    if let Ok(g) = fs::read_to_string(&opt.dotfile) {
        process_dot(&g, &opt.name);
    } else {
        error!("Failed to read dot file: {}", &opt.dotfile);
    }

}

fn process_dot(dot: &str, name: &str) {
    //before we parse the dot string we must remove all comments
    //we will remove all text starting with // up to the end of the line
    let dot = dot.lines()
        .map(|line| {
            if let Some(pos) = line.find("//") {
                &line[0..pos]
            } else {
                line
            }
        })
        .collect::<Vec<&str>>()
        .join("\n");


    //
    match ast::Graph::read_dot(&dot) {
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

            info!("Project folder created at: {}", base_folder_path.display());
            info!("Source folder created at: {}", source_folder_path.display());
            info!("Actor folder created at: {}", actor_folder_path.display());

            ///////////
            ///////////
            //folders done now extract the project details
            ///////////
            match extract_details::extract_project_model(name, canonical::Graph::from(ast)) {
                Ok(project_model) => {
                    info!("Project model extracted");
                    //last write out the project files
                    if let Err(e) = write_project_files(project_model, base_folder_path, source_folder_path, actor_folder_path) {
                        error!("Partial project files written: {}", e);
                    }
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
                       , folder_base: PathBuf
                       , folder_src: PathBuf
                       , folder_actor: PathBuf) -> Result<(), Box<dyn Error>> {

   let cargo = folder_base.join("Cargo.toml");
   fs::write(cargo, templates::CargoTemplate { name: &pm.name }.render()?)?;

   let gitignore = folder_base.join(".gitignore");
   fs::write(gitignore, templates::GitIgnoreTemplate {}.render()?)?;

   let args_rs = folder_src.join("args.rs");
   fs::write(args_rs, templates::ArgsTemplate {}.render()?)?;

    #[cfg(test)]
        let test_only = "#[allow(unused)]";
    #[cfg(not(test))]
        let test_only = "";

   let main_rs = folder_src.join("main.rs");
   let mut actor_mods:Vec<String> =  pm.actors.iter().map(|f| f.mod_name.clone()).collect();
   actor_mods.sort();
   actor_mods.dedup();

   fs::write(main_rs, templates::MainTemplate {
        test_only,
        actor_mods,
        channels: &pm.channels,
        actors: &pm.actors,
   }.render()?)?;

   for actor in pm.actors {
       let actor_file_rs = folder_actor.join(Path::new(&actor.mod_name)).with_extension("rs");

       //need list of unique message types, do we dedup here??
       let mut my_struct_use:Vec<String> = actor.rx_channels
            .iter()
            .map(|f| format!("use crate::actor::{}::{}",f[0].from_mod, f[0].message_type))
            .collect();
       my_struct_use.sort();
       my_struct_use.dedup();

       //NOTE: if we use a struct of the same name we assume it is the same and
       //      do not define it again.
       let mut my_struct_def:Vec<String> = actor.tx_channels
           .iter()
           .filter(|f| actor.rx_channels.iter().all(|g| !g[0].message_type.eq(&f[0].message_type)))
           .map(|f| format!("{}pub(crate) struct {}", derive_block(f[0].copy), f[0].message_type))
           .collect();
       my_struct_def.sort();
       my_struct_def.dedup();





       let full_driver_block = build_driver_block(&actor);
       let full_process_example_block = build_process_block(&actor);

       //build monitor lists
       let rx_monitor_defs = monitor_defs("rx",&actor.rx_channels);
       let tx_monitor_defs = monitor_defs("tx",&actor.tx_channels);

       fs::write(actor_file_rs, templates::ActorTemplate {
               note_for_the_user: "//TODO: ", //do not change, this is not for you
               display_name: actor.display_name,
               has_bundles: actor.rx_channels.iter().any(|f| f.len()>1) || actor.tx_channels.iter().any(|f| f.len()>1),
               rx_channels: actor.rx_channels,
               tx_channels: actor.tx_channels,
               rx_monitor_defs,
               tx_monitor_defs,
               full_driver_block,
               full_process_example_block,
               message_types_to_use: my_struct_use,
               message_types_to_define: my_struct_def,
       }.render()?)?;
   }
   Ok(())
}

fn build_process_block(_actor: &Actor) -> String {
    let full_process_example_block = String::new();
    // TODO: some examples for each field how to read and write.
    full_process_example_block
}

fn build_driver_block(actor: &Actor) -> String {

    let mut at_least_every: Option<String> = None;
    let mut at_most_every: Option<String> = None;

    let mut avail_units: Vec<String> = Vec::new();
    let mut vacant_units: Vec<String> = Vec::new();
    let mut other_units: Vec<String> = Vec::new();

    actor.driver.iter().for_each(|f| {
        match f {
            ActorDriver::AtLeastEvery(d) => {
                at_least_every = Some(format!("monitor.relay_stats_periodic(Duration::from_millis({:?})).await",d.as_millis()));
            }
            ActorDriver::AtMostEvery(d) => {
                at_most_every = Some(format!("monitor.relay_stats_periodic(Duration::from_millis({:?})).await",d.as_millis()));
            }
            ActorDriver::EventDriven(t) => {
                let mut each: Vec<String> = t.iter().map(|v| {

                    let girth = if let Some(g) = actor.rx_channels.iter()
                            .find(|f| f[0].name == v[0])
                            .map(|f| f.len()) { g } else { 1 };

                    //validate the name
                    if actor.rx_channels.iter().all(|f| f[0].name != v[0]) {
                        warn!("Failed to find channel: {}, please fix dot file node label for actor: {}", v[0], actor.display_name);
                    }

                    //2 may want the default channels_count or this may be a single
                    //  we ensure girth is 1 to confirm this choice.
                    if v.len()==2 && 1==girth {
                        format!("monitor.wait_avail_units(&mut {}_rx,{}).await", v[0], v[1])
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
                        format!("monitor.wait_avail_units_bundle(&mut {}_rx,{},{}).await", v[0], v[1], channels_count)
                    }
                }).collect();
                avail_units.append(&mut each);
            }
            ActorDriver::CapacityDriven(t) => {
                let mut each: Vec<String> = t.iter().map(|v| {

                    //validate the name
                    if actor.tx_channels.iter().all(|f| f[0].name != v[0]) {
                        warn!("Failed to find channel: {}, please fix dot file node label for actor: {}", v[0], actor.display_name);
                    }


                    if v.len() == 2 {
                        format!("monitor.wait_vacant_units(&mut {}_tx,{}).await", v[0], v[1])
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
                        format!("monitor.wait_vacant_units_bundle(&mut {}_tx,{},{}).await", v[0], v[1], channels_count)
                    }
                }).collect();
                vacant_units.append(&mut each);
            }
            ActorDriver::Other(t) => {
                let mut each: Vec<String> = t.iter().map(|name| {
                    format!("//monitor.shutdown_wrapper({}()).await", name)
                }).collect();
                other_units.append(&mut each);
            }
        }
    });

    let mut full_driver_block = String::new();

    if let Some(t) = at_least_every {
        //this block must be a wrapping select around the others

        //let delay_future = Delay::new(Duration::from_millis(3));
        //select! {
         //   _ = delay_future.fuse() => {},
         //   _ = monitor.wait_avail_units(&mut rx,count).fuse() => {},
        //}

        full_driver_block.push_str(&t);
        full_driver_block.push(';');
        full_driver_block.push('\n');
    }


    if let Some(t) = at_most_every {
        full_driver_block.push_str(&t);
        full_driver_block.push(';');
        full_driver_block.push('\n');
    }


    //TODO: these all need to be put into an array when we need the at_least_every

    avail_units.iter().for_each(|t| {
        full_driver_block.push_str(&t);
        full_driver_block.push(';');
        full_driver_block.push('\n');
    });

    vacant_units.iter().for_each(|t| {
        full_driver_block.push_str(&t);
        full_driver_block.push(';');
        full_driver_block.push('\n');
    });

    other_units.iter().for_each(|t| {
        full_driver_block.push_str(&t);
        full_driver_block.push(';');
        full_driver_block.push('\n');
    });

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
             result.push(format!("{}_{}.def_slice()",c[0].name,direction).into());
         } else {
             //NOTE: once we have simple expressions of generic constants to sum arrays this
             //     code can be removed
             c.iter()
                 .enumerate()
                 .for_each(|(i,f)| { //NOTE: each requies an index
                 result.push(format!("{}_{}[{}]",f.name,direction,i).into());
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
        true =>  "#[derive(Default,Clone,Copy)]\n",
        false => "#[derive(Default)]\n",
    }
}


#[cfg(test)]
mod tests {
    use std::{env, fs};
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::str::FromStr;
    use flexi_logger::{Logger, LogSpecBuilder};
    use log::{error, info, LevelFilter};
    use crate::{process_dot};




    fn build_and_parse(test_name: &str, g: &str, clean: bool)  {

        let level = "warn";
        if let Ok(s) = LevelFilter::from_str(&level) {
            let mut builder = LogSpecBuilder::new();
            builder.default(s); // Set the default level
            let log_spec = builder.build();

            //turn on for debug
            if false {
                Logger::with(log_spec)
                    .format(flexi_logger::colored_with_thread)
                    .start().expect("Logger did not start");
            }

        } else {
            eprint!("Warning: Logger initialization failed with bad level: {:?}. There will be no logging.", &level);
        }

        //ensure test_run folder exists
        let working_path = PathBuf::from("test_run");
        if let Err(e) = fs::create_dir_all(&working_path) {
            error!("Failed to create test_run directory: {}", e);
            return;
        }

        //move to our test_run folder to ensure we do not generate test code on top of our self
        match env::set_current_dir("test_run") {
            Ok(_) => {
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

                /////
                process_dot(g, test_name);


                const DO_COMPILE_TEST:bool = false;

                if DO_COMPILE_TEST {
                  let build_me = PathBuf::from(test_name);

                     let build_me_absolute = env::current_dir().unwrap().join(build_me).canonicalize().unwrap();
                    ////
                    let mut output = Command::new("cargo")
                        .arg("build")
                        .arg("--manifest-path")
                        .arg(build_me_absolute.join("Cargo.toml").to_str().unwrap()) // Ensure this path points to your generated Cargo.toml
                        .current_dir(build_me_absolute.clone())
                        .stdout(Stdio::inherit()) // This line ensures that stdout from the command is printed directly to the terminal
                        .stderr(Stdio::inherit()) // This line ensures that stderr is also printed directly to the terminal
                        .spawn()
                        .expect("failed to execute process");
                    let output = output.wait().expect("failed to wait on child");

                    assert!(output.success());
                }

            }
            Err(e) => {
                panic!("Failed to change directory to test_run: {}", e);
            }
        }
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

    ConfigLoader -> IMAPClient [label="name::cfg <ServerDetails> Imap server details\nemail, password"];
    ImapClient -> EmailFilter [label="name::email <Email> Emails\nRaw email data"];
    EmailFilter -> SpamAssassin [label="name::content <EmailBody> Email content\nFor SpamAssassin analysis"];
    SpamAssassin -> EmailFilter [label="name::verdict <SpamScore> Spam verdict\nSpam or Ham"];
    EmailFilter -> EmailMover [label="name::spam <SpamBody> Spam emails\nIdentified spam messages"];
    EmailMover -> Logger [label="name::email_log <SuccessLog> Move operations\nSuccess or failure logs"];
    ImapClient -> Logger [label="name::con_log <Connections> Connection logs\nSuccess or failure"];
    EmailFilter -> Logger [label="name::filt_log <FilterLog> Filtering logs\nProcessed emails and results"];

    edge [color=blue];
    node [style=filled, color=lightgrey];
}
        "#;

        build_and_parse("unnamed1", g, true);
    }

    #[test]
    fn test_pbft_project() {
        let g = r#"
digraph PBFTDemo {
     Client [label="Initiates requests\nmod::client AtLeastEvery(5sec)"];
     Primary [label="Orders requests and initiates consensus\nmod::primary OnEvent(original_request:1||feedback:1)"];

     Client -> Primary [label="name::original_request <TransactionRequest> >>PeekCopy #20"];
     Replica1 [label="Participates in consensus\nmod::replica OnEvent(pbft_message:1)"];
     Replica2 [label="Participates in consensus\nmod::replica OnEvent(pbft_message:1)"];
     Replica3 [label="Participates in consensus\nmod::replica OnEvent(pbft_message:1)"];

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
        build_and_parse("pbft_demo", g, false);
    }
}




