//! This module facilitates the deployment of SteadyState projects on Linux and their management as systemd services.
//!
//! The module provides functionalities to create, install, and uninstall systemd service files, enabling seamless
//! service management for SteadyState applications.

use std::{env, fs};
use std::process::Command;
use log::*;
use std::path::{Path, PathBuf};
use crate::abstract_executor;
use crate::dot::FrameHistory;

/// A builder for configuring and creating a systemd service manager.
#[derive(Clone)]
pub struct SystemdBuilder {
    service_name: String,
    service_user: String,
    service_file_default_folder: String,
    service_executable_folder: String,
    on_boot: bool,
    secrets: Vec<String>,
    after: String,
    restart: String,
    wanted_by: String,
    description: String,
}

impl SystemdBuilder {
    /// Creates a new `SystemdBuilder` with the given executable name and user.
    ///
    /// # Arguments
    ///
    /// * `service_executable_name` - The name of the service executable.
    /// * `service_user` - The user under which the service will run.
    ///

    pub fn new(service_executable_name: String, service_user: String) -> Self {
        SystemdBuilder {
            service_executable_folder: "/usr/local/bin".to_string(),
            service_file_default_folder: "/etc/systemd/system".to_string(),
            service_name: service_executable_name.clone(),
            secrets: Vec::new(),
            on_boot: true,
            description: format!("steady_state:{}", service_executable_name),
            after: "network.target".to_string(),
            wanted_by: "multi-user.target".to_string(),
            restart: "always".to_string(),
            service_user,
        }
    }

    /// Adds a secret to the service configuration.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the secret.
    /// * `absolute_file` - The absolute path to the secret file.
    ///

    pub fn with_secret(&self, name: String, absolute_file: String) -> Self {
        let mut result = self.clone();
        result.secrets.push(format!("{}:/{}", name, absolute_file));
        result
    }

    /// Configures the service to start on boot.
    ///
    /// # Arguments
    ///
    /// * `on_boot` - A boolean indicating whether the service should start on boot.
    ///

    pub fn with_on_boot(&self, on_boot: bool) -> Self {
        let mut result = self.clone();
        result.on_boot = on_boot;
        result
    }

    /// Sets the description of the service.
    ///
    /// # Arguments
    ///
    /// * `description` - The description of the service.
    ///

    pub fn with_description(&self, description: String) -> Self {
        let mut result = self.clone();
        result.description = description;
        result
    }

    /// Sets the `After` directive for the service.
    ///
    /// # Arguments
    ///
    /// * `after` - The service or target that this service should start after.
    ///

    pub fn with_after(&self, after: String) -> Self {
        let mut result = self.clone();
        result.after = after;
        result
    }

    /// Sets the `WantedBy` directive for the service.
    ///
    /// # Arguments
    ///
    /// * `wanted_by` - The target that wants this service.
    ///

    pub fn with_wanted_by(&self, wanted_by: String) -> Self {
        let mut result = self.clone();
        result.wanted_by = wanted_by;
        result
    }

    /// Sets the `Restart` directive for the service.
    ///
    /// # Arguments
    ///
    /// * `restart` - The restart policy for the service.
    ///

    pub fn with_restart(&self, restart: String) -> Self {
        let mut result = self.clone();
        result.restart = restart;
        result
    }

    /// Sets the user under which the service will run.
    ///
    /// # Arguments
    ///
    /// * `service_user` - The user for the service.
    ///

    pub fn with_service_user(&self, service_user: String) -> Self {
        let mut result = self.clone();
        result.service_user = service_user;
        result
    }

    /// Sets the name of the service.
    ///
    /// # Arguments
    ///
    /// * `service_name` - The name of the service.
    ///

    pub fn with_service_name(&self, service_name: String) -> Self {
        let mut result = self.clone();
        result.service_name = service_name;
        result
    }

    /// Sets the default folder for the service file.
    ///
    /// # Arguments
    ///
    /// * `service_file_default_folder` - The default folder for the service file.
    ///

    pub fn with_service_file_default_folder(&self, service_file_default_folder: String) -> Self {
        let mut result = self.clone();
        result.service_file_default_folder = service_file_default_folder;
        result
    }

    /// Sets the folder for the service executable.
    ///
    /// # Arguments
    ///
    /// * `service_executable_folder` - The folder for the service executable.
    ///

    pub fn with_service_executable_folder(&self, service_executable_folder: String) -> Self {
        let mut result = self.clone();
        result.service_executable_folder = service_executable_folder;
        result
    }

    /// Builds and returns a `SystemdServiceManager`.
    ///

    pub fn build(self) -> SystemdServiceManager {
        SystemdServiceManager {
            service_file_name: format!("{}/{}.service", &self.service_file_default_folder, &self.service_name),
            service_executable: format!("{}/{}", &self.service_executable_folder, &self.service_name),
            service_name: self.service_name,
            service_user: self.service_user,
            on_boot: self.on_boot,
            secrets: self.secrets,
            after: self.after,
            restart: self.restart,
            wanted_by: self.wanted_by,
            description: self.description,
        }
    }
}

/// Manages a systemd service for a SteadyState project.
///
/// The `SystemdServiceManager` struct is used to manage the creation, configuration, and control
/// of a systemd service associated with a SteadyState project. It contains various fields to specify
/// the service details and configuration options.
pub struct SystemdServiceManager {
    /// The name of the systemd service.
    ///
    /// This name is used to identify the service within systemd.
    pub service_name: String,

    /// The user under which the service runs.
    ///
    /// **Note:** Never share this user with other services to maintain security isolation.
    pub service_user: String,

    /// The file name of the systemd service unit file.
    ///
    /// This file contains the configuration for the service and is typically located in `/etc/systemd/system/`.
    pub service_file_name: String,

    /// The executable that the service will run.
    ///
    /// This is the path to the binary or script that will be executed when the service starts.
    pub service_executable: String,

    /// Indicates whether the service should start on boot.
    ///
    /// If `true`, the service will be configured to start automatically when the system boots.
    pub on_boot: bool,

    /// A list of secrets required by the service.
    ///
    /// These secrets are used to provide sensitive information to the service in a secure manner.
    secrets: Vec<String>,

    /// Specifies the units that this service should start after.
    ///
    /// This is used to ensure that this service starts only after the specified units have started.
    pub after: String,

    /// The restart policy for the service.
    ///
    /// This field defines the conditions under which systemd should attempt to restart the service.
    pub restart: String,

    /// Specifies the target unit that should want this service.
    ///
    /// This field is typically used to associate the service with a particular target, such as `multi-user.target`.
    pub wanted_by: String,

    /// A description of the service.
    ///
    /// This description provides information about the service and its purpose, and is often displayed by systemd tools.
    pub description: String,
}


impl SystemdServiceManager {
    /// Checks if the platform setup is appropriate for managing a systemd service.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
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

    /// Installs the service, optionally starting it immediately.
    ///
    /// # Arguments
    ///
    /// * `start_now` - A boolean indicating whether to start the service immediately.
    /// * `start_string` - The command to start the service.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    ///

    pub fn install(&self, start_now: bool, start_string: String) -> Result<(), Box<dyn std::error::Error>> {
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
            // Could be replaced with a more dynamic or builder approach based on distribution
            let useradd_command = "useradd";
            let useradd_args = ["-r", "-s", "/usr/sbin/nologin", &self.service_user];

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
            info!("Enabled '{}' service to start on boot", self.service_name);
        }

        if start_now {
            Command::new("systemctl")
                .args(["start", &self.service_name])
                .status()?;
            info!("Started '{}' service", self.service_name);
            info!("To debug try: journalctl -u {}", self.service_name);
        }

        Ok(())
    }

    /// Uninstalls the service.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    ///

    pub fn uninstall(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.check_platform_setup()?;

        // Stop the service
        Command::new("systemctl")
            .args(["stop", &self.service_name])
            .status()?;
        info!("Stopped '{}' service", self.service_name);

        // Disable the service
        Command::new("systemctl")
            .args(["disable", &self.service_name])
            .status()?;
        info!("Disabled '{}' service from starting on boot", self.service_name);

        // Remove the systemd service file
        std::fs::remove_file(Path::new(&self.service_file_name))?;
        info!("Removed the systemd service file: {}", self.service_file_name);

        // Reload the systemd daemon
        Command::new("systemctl")
            .arg("daemon-reload")
            .status()?;
        info!("Reloaded the systemd daemon");

        // Remove the executable
        std::fs::remove_file(Path::new(&self.service_executable))?;
        info!("Removed the executable: {}", self.service_executable);

        Command::new("userdel")
            .arg(&self.service_user)
            .status()?;
        info!("Deleted the service user '{}'", self.service_user);

        Ok(())
    }

    /// Creates the systemd service file.
    ///
    /// # Arguments
    ///
    /// * `start_string` - The command to start the service.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    fn create_service_file(&self, start_string: String) -> Result<(), String> {
        let mut load_creds = String::new();
        if !self.secrets.is_empty() {
            load_creds.push_str("# systemd's LoadCredential Option (systemd v246+)\n");
        }
        for secret in &self.secrets {
            load_creds.push_str(&format!("LoadCredential={}\n", secret));
        }

        let service_content = format!(
            r#"[Unit]
Description={}
After={}

[Service]
{}
ExecStart={}
User={}
Restart={}

[Install]
WantedBy={}
"#,
            self.description, self.after, load_creds, start_string, self.service_user, self.restart, self.wanted_by
        );

        info!("Write service content to file: {}", service_content);

        let filename = (&self.service_file_name).into();
        abstract_executor::block_on(async move {
            match FrameHistory::truncate_file(filename, service_content.as_bytes().into()).await {
                Ok(_) => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        })
    }
}

