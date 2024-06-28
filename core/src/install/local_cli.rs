//! This module provides functionality to build and install local CLI tools for the SteadyState project.
//! It supports both system-wide and user-local installations and allows for custom installation directories.

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use log::*;
use dirs; // Ensure `dirs` crate is added to your `Cargo.toml`

/// A builder for creating and installing local CLI tools.
#[derive(Clone)]
pub struct LocalCLIBuilder {
    pub(crate) path: String,
    pub(crate) system_wide: bool,
    pub(crate) install_dir: PathBuf,
}

impl LocalCLIBuilder {
    /// Creates a new `LocalCLIBuilder`.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to the executable.
    /// * `system_wide` - A boolean indicating if the installation should be system-wide.
    ///
    /// # Returns
    ///
    /// A new `LocalCLIBuilder` instance.
    pub fn new(path: String, system_wide: bool) -> Self {
        let install_dir = if system_wide {
            PathBuf::from("/usr/local/bin")
        } else {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".local/bin")
        };

        LocalCLIBuilder {
            path,
            system_wide,
            install_dir,
        }
    }

    /// Sets a custom installation directory.
    ///
    /// # Arguments
    ///
    /// * `custom_install_dir` - The custom installation directory.
    ///
    /// # Returns
    ///
    /// The `LocalCLIBuilder` instance with the updated installation directory.
    ///
    /// # Warning
    ///
    /// This should only be used if you know you need it for some custom distribution.
    pub fn with_custom_location(mut self, custom_install_dir: PathBuf) -> Self {
        let in_path = env::var_os("PATH")
            .map_or(false, |paths| paths.to_string_lossy().contains(&*custom_install_dir.to_string_lossy()));
        self.system_wide = false;
        if in_path {
            self.install_dir = custom_install_dir;
        } else {
            warn!("Custom install directory is not in the PATH");
        }
        self
    }

    /// Builds and installs the CLI tool.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub fn build(&self) -> std::io::Result<()> {
        let current_exe = env::current_exe()?;
        info!("Current executable path: {:?}", current_exe);
        info!("path: {}", self.path);

        fs::create_dir_all(&self.install_dir)?;

        let dest_path = self.install_dir.join(
            current_exe.file_name().ok_or(std::io::Error::new(std::io::ErrorKind::NotFound, "Executable name not found"))?
        );
        fs::copy(&current_exe, &dest_path)?;

        let mut perms = fs::metadata(&dest_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest_path, perms)?;

        info!("Executable installed to: {:?}", dest_path);

        Ok(())
    }
}
