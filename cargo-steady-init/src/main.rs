mod extract_details;
mod args;
mod templates;

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use askama::Template;
use dot_parser::{ast, canonical};
#[allow(unused_imports)]
use log::*;
use structopt::StructOpt;
use crate::args::Args;
use crate::templates::*;

#[derive(Default)]
struct ProjectModel {
    pub(crate) name: String,
    pub(crate) actors: Vec<Actor>,
    pub(crate) channels: Vec<Channel>,
}

fn main() {
    let opt = Args::from_args();
    if let Err(e) = steady_state::init_logging(&opt.loglevel) {
        //do not use logger to report logger could not start
        eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
    }

    if let Ok(g) = fs::read_to_string(&opt.dotfile) {
        process_dot(&g, &opt.name);
    } else {
        error!("Failed to read dot file: {}", &opt.dotfile);
    }

}

fn process_dot(dot: &str, name: &str) {
    //
    match ast::Graph::read_dot(dot) {
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
   fs::write(main_rs, templates::MainTemplate {
        test_only,
        channels: &pm.channels,
        actors: &pm.actors,
   }.render()?)?;

    warn!("write actors: {:?}",pm.actors.len());
   for actor in pm.actors {
       let actor_rs = folder_actor.join(actor.mod_name + ".rs");

       //need list of unique message types, do we dedup here??
       let mut my_struct_use:Vec<String> = actor.rx_channels
            .iter()
            .map(|f| format!("use crate::actor::{}::{}",f.from_mod, f.message_type))
            .collect();
       my_struct_use.sort();
       my_struct_use.dedup();

       let mut my_struct_def:Vec<String> = actor.tx_channels
           .iter()
           .map(|f| format!("{}pub(crate) struct {}", derive_block(f.copy), f.message_type))
           .collect();
       my_struct_def.sort();
       my_struct_def.dedup();


       fs::write(actor_rs, templates::ActorTemplate {
               note_for_the_user: "TODO", //do not change, this is not for you
               display_name: actor.display_name,
               rx_channels: actor.rx_channels,
               tx_channels: actor.tx_channels,
               message_types_to_use: my_struct_use,
               message_types_to_define: my_struct_def,
       }.render()?)?;
   }
   Ok(())
}

fn derive_block(copy: bool) -> &'static str {
    match copy {
        true =>  "#[derive(Default,Copy)]\n",
        false => "#[derive(Default)]\n",
    }
}


#[cfg(test)]
mod tests {
    use std::{env, fs};
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use log::{error, info};
    use crate::process_dot;

    #[test]
    fn test_unnamed1_project() {

        if let Err(e) = steady_state::init_logging("info") {
            //do not use logger to report logger could not start
            eprint!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
        }


        let do_compile_test = true;

        let g = r#"
        digraph G {
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
        //ensure test_run folder exists
        let working_path = PathBuf::from("test_run");
        if let Err(e) = fs::create_dir_all(&working_path) {
            error!("Failed to create test_run directory: {}", e);
            return;
        }

        //move to our test_run folder to ensure we do not generate test code on top of ourself
        match env::set_current_dir("test_run") {
            Ok(_) => {
                let current_dir = env::current_dir().expect("Failed to get current directory");
                println!("Current working directory is: {:?}", current_dir);
                //NOTE: must delete the unnamed1 folder if it exists
                if let Err(e) = fs::remove_dir_all("unnamed1") {
                    error!("Failed to remove test_run/unnamed1 directory: {}", e);
                } else {
                    info!("Removed test_run/unnamed1 directory");
                }

                /////
                process_dot(g, "unnamed1");
                let build_me = PathBuf::from("unnamed1");
                let build_me_absolute = env::current_dir().unwrap().join(build_me).canonicalize().unwrap();

                if do_compile_test {

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
}




