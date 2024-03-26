use std::{env, fs};
use std::error::Error;
use std::process::Command;
use log::*;
#[allow(unused_imports)]
use std::io::Write;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use nuclei::{drive, Handle};

#[allow(unused_imports)]
use futures::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
#[allow(unused_imports)]
use std::fs::{create_dir_all, File, OpenOptions};
use bastion::run;
use bytes::BytesMut;
use nuclei::Task;
use crate::{dot, util};
use crate::dot::FrameHistory;

pub struct SystemdServiceManager {
    pub service_name: String,
    pub service_user: String, // NEVER share this user with other services
    pub service_file_name: String,
    pub service_executable: String,
    pub on_boot: bool,
    secrets: Vec<String>,
    pub after: String,
    pub restart: String,
    pub wanted_by: String,
    pub description: String,
}

//TODO: this needs to be the builder pattern soon,

struct SystemdServiceManagerBuilder {
    service_name: String,
    service_user: String,
    service_file_name: String,
    service_executable: String,
    on_boot: bool,
    secrets: Vec<String>,
    after: String,
    restart: String,
    wanted_by: String,
    description: String,
}

impl SystemdServiceManager {

    pub fn new(service_executable_name: String, service_user: String) -> Self {
        SystemdServiceManager {
            //TODO: add support for other folders based on use case
            service_file_name: format!("/etc/systemd/system/{}.service", service_executable_name),
            //TODO: add support for diffent binary locations
            service_executable: format!("/usr/local/bin/{}",service_executable_name),
            on_boot: true,
            secrets: Vec::new(),
            after: "network.target".to_string(),
            restart: "always".to_string(),
            wanted_by: "multi-user.target".to_string(),
            service_name: service_executable_name.clone(),
            service_user,
            description: format!("steady_state:{}",service_executable_name).into(),
        }
    }

    pub fn add_secret(&mut self, name: String, absolute_file: String
    ) {
        //absolute_file probably should start with /etc/secrets/
        self.secrets.push(format!("{}:/{}", name, absolute_file));
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
                .args(useradd_args)
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
                        .args(["enable", &self.service_name])
                        .status()?;
            info!("Enabled '{}' service to start on boot",self.service_name);
        }

        if start_now {
            Command::new("systemctl")
                        .args(["start", &self.service_name])
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
                    .args(["stop", &self.service_name])
                    .status()?;
        info!("Stopped '{}' service",self.service_name);

        // Disable the service
        Command::new("systemctl")
                    .args(["disable", &self.service_name])
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


    fn create_service_file(&self, start_string: String) -> Result<(), String> {

         let mut load_creds = String::new();
         if !self.secrets.is_empty() {
            load_creds.push_str("# systemd's LoadCredential Option (systemd v246+)\n");
         }
         for secret in &self.secrets {
                load_creds.push_str(&format!("LoadCredential={}\n", secret));
         }

        let service_content = format!(r#"[Unit]
Description={}
After={}

[Service]
{}
ExecStart={}
User={}
Restart={}

[Install]
WantedBy={}
"#
 ,self.description, self.after, load_creds, start_string, self.service_user
 ,self.restart, self.wanted_by  );

        info!("write service content to file: {}",service_content);

        match run!(FrameHistory::write_file((&self.service_file_name).into(), service_content.as_bytes().into())) {
            Ok(_) => Ok(()),
            Err(e) => Err(e.to_string())
        }
    }

}

