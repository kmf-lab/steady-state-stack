use std::{env, fs};
use std::process::Command;
use log::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use nuclei::{drive, Handle};
// new Proactive IO
use nuclei::*;
use futures::io::SeekFrom;
use futures::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use std::fs::{create_dir_all, File, OpenOptions};
use crate::steady_state::util;

pub struct SystemdManager {
    pub service_name: String,
    pub service_user: String, // NEVER share this user with other services
    pub service_file_name: String,
    pub service_executable: String,
    pub on_boot: bool,
    secrets: Vec<String>,
    pub after: String,
    pub restart: String,
    pub wanted_by: String,
}

impl SystemdManager {

    pub fn add_secret(&mut self, name: String, absolute_file: String
                         ) {
        //absolute_file probably should start with /etc/secrets/
        self.secrets.push(format!("{}:/{}", name, absolute_file));
    }


    pub fn new(service_name: String, service_user: String) -> Self {
        SystemdManager {
            //TODO: add support for other folders based on use case
            service_file_name: format!("/etc/systemd/system/{}.service", service_name),
            //TODO: add support for diffent binary locations
            service_executable: format!("/usr/local/bin/{}",service_name),
            on_boot: true,
            secrets: Vec::new(),
            after: "network.target".to_string(),
            restart: "always".to_string(),
            wanted_by: "multi-user.target".to_string(),
            service_name,
            service_user,
        }
    }

    fn check_platform_setup(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Check for systemd via systemctl
        if !Command::new("systemctl")
            .arg("--version")
            .status()?.success() {
            return Err("Systemd is not installed".into());
        }
        // Check for elevated permissions
        if env::var("USER").unwrap_or_default() != "root" {
            return Err("This command needs to be run with elevated permissions".into());
        }
        info!("Running with elevated permissions");
        Ok(())
    }

    // to_cli_string(self.service_executable,cmd_line_arg)
    pub fn install(&self, start_now:bool, start_string: String) -> Result<(), Box<dyn std::error::Error>> {
        self.check_platform_setup()?;

        // Detect the current executable's path
        let current_exe = env::current_exe()?;
        info!("Current executable path: {:?}", current_exe);

        // Copy the executable to /usr/local/bin/
        fs::copy(&current_exe, Path::new(&self.service_executable))?;
        info!("Copied the executable to: {}", self.service_executable);

        let status = Command::new("id")
            .arg("-u")
            .arg(&self.service_user)
            .status()?;

        if !status.success() {
            //TODO: This can be replaced with a more dynamic approach based on the distribution
            let useradd_command = "useradd";
            let useradd_args = ["-r", "-s", "/usr/sbin/nologin", &self.service_user];

            ////
            Command::new(useradd_command)
                .args(&useradd_args)
                .status()?;
            info!("Created the service user '{}'", self.service_user);
        } else {
            info!("Service user '{}' already exists", self.service_user);
        }

        // Create the service file
        self.create_service_file(start_string)?;

        // Reload the systemd daemon
        Command::new("systemctl")
                    .arg("daemon-reload")
                    .status()?;
        info!("Reloaded the systemd daemon");

        if self.on_boot {
            // Enable the service to start on boot
            Command::new("systemctl")
                        .args(&["enable", &self.service_name])
                        .status()?;
            info!("Enabled '{}' service to start on boot",self.service_name);
        }

        if start_now {
            Command::new("systemctl")
                        .args(&["start", &self.service_name])
                        .status()?;
            info!("Started '{}' service",self.service_name);
            info!("To debug try: journalctl -u {}",self.service_name);
        }

        Ok(())
    }

    pub fn uninstall(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.check_platform_setup()?;

        // Stop the service
        Command::new("systemctl")
                    .args(&["stop", &self.service_name])
                    .status()?;
        info!("Stopped '{}' service",self.service_name);

        // Disable the service
        Command::new("systemctl")
                    .args(&["disable", &self.service_name])
                    .status()?;
        info!("Disabled '{}' service from starting on boot",self.service_name);

        // Remove the systemd service file
        std::fs::remove_file(Path::new(&self.service_file_name))?;
        info!("Removed the systemd service file: {}",self.service_file_name);

        // Reload the systemd daemon
        Command::new("systemctl")
                        .arg("daemon-reload")
                        .status()?;
        info!("Reloaded the systemd daemon");

        // Remove the executable
        std::fs::remove_file(Path::new(&self.service_executable))?;
        info!("Removed the executable: {}",self.service_executable);

        Command::new("userdel")
                    .arg(&self.service_user)
                    .status()?;
        info!("Deleted the service user '{}'",self.service_user);

        Ok(())
    }


    fn create_service_file(&self, start_string: String) -> Result<(), Box<dyn std::error::Error>> {

         let mut load_creds = String::new();
         if self.secrets.len() > 0 {
            load_creds.push_str("# systemd's LoadCredential Option (systemd v246+)\n");
         }
         for secret in &self.secrets {
                load_creds.push_str(&format!("LoadCredential={}\n", secret));
         }

        let service_content = format!(r#"[Unit]
Description=SOME DESCRIPTION
After={}

[Service]
{}
ExecStart={}
User={}
Restart={}

[Install]
WantedBy={}
"#
 , self.after
 , load_creds, start_string, self.service_user
          , self.restart, self.wanted_by   );

        let file = OpenOptions::new()
            .create(true)
            .open(&self.service_file_name)?;
            let _ =drive(util::all_to_file_async(file, service_content.as_bytes()));
        info!("Created the systemd service file: {}",self.service_file_name);

        Ok(())
    }


}

