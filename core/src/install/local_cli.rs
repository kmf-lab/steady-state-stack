//! This module provides functionality to build and install local CLI tools for the SteadyState project.
//! It supports both system-wide and user-local installations and allows for custom installation directories.

use std::env;
use std::fs;
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
    /// Creates a new `LocalCLIBuilder` instance with platform-specific installation directories.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to the executable or a related configuration path.
    /// * `system_wide` - A boolean indicating if the installation should be system-wide.
    ///
    /// # Returns
    ///
    /// A new `LocalCLIBuilder` instance configured for the target platform.
    ///
    /// # Platform-Specific Behavior
    ///
    /// - **Linux (Unix-like systems):**
    ///   - System-wide: `/usr/local/bin`
    ///   - User-local: `~/.local/bin`
    /// - **Windows:**
    ///   - System-wide: `C:\Program Files\SteadyState`
    ///   - User-local: `%USERPROFILE%\.local\bin`
    ///
    /// # Notes
    ///
    /// - On Windows, system-wide installation requires administrative privileges.
    /// - The user may need to add the installation directory to their PATH manually.
    pub fn new(path: String, system_wide: bool) -> Self {
        let install_dir = if system_wide {
            if cfg!(unix) {
                PathBuf::from("/usr/local/bin")
            } else if cfg!(windows) {
                PathBuf::from(r"C:\Program Files\SteadyState")
            } else {
                panic!("Unsupported platform");
            }
        } else {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            if cfg!(unix) {
                home.join(".local/bin")
            } else if cfg!(windows) {
                home.join(".local").join("bin")
            } else {
                panic!("Unsupported platform");
            }
        };

        LocalCLIBuilder {
            path,
            system_wide,
            install_dir,
        }
    }

    /// Sets a custom installation directory, overriding the default.
    ///
    /// # Arguments
    ///
    /// * `custom_install_dir` - The custom installation directory as a `PathBuf`.
    ///
    /// # Returns
    ///
    /// The updated `LocalCLIBuilder` instance.
    ///
    /// # Behavior
    ///
    /// - If the custom directory is in the system's PATH, it is set as the `install_dir`.
    /// - If not in the PATH, a log message is emitted, and the directory is still set.
    /// - This method disables system-wide installation (`system_wide` is set to `false`).
    ///
    /// # Warning
    ///
    /// Use this only if you need a custom distribution setup. The user must ensure the directory
    /// is in the PATH if they want the CLI tool to be accessible without full path specification.
    pub fn with_custom_location(mut self, custom_install_dir: PathBuf) -> Self {
        self.system_wide = false;
        if let Ok(path_var) = env::var("PATH") {
            let paths: Vec<PathBuf> = env::split_paths(&path_var).collect();
            if paths.contains(&custom_install_dir) {
                self.install_dir = custom_install_dir;
            } else {
                trace!("Custom install directory {:?} is not in the PATH", custom_install_dir);
                self.install_dir = custom_install_dir;
            }
        } else {
            info!("PATH environment variable not found");
            self.install_dir = custom_install_dir;
        }
        self
    }

    /// Builds and installs the CLI tool to the specified installation directory.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or an I/O error if the operation fails.
    ///
    /// # Behavior
    ///
    /// - Copies the current executable to the `install_dir` with its original filename.
    /// - On Unix-like systems, sets executable permissions (0o755).
    /// - Creates the installation directory if it doesn't exist.
    /// - Logs the installation process using the `log` crate.
    ///
    /// # Platform-Specific Notes
    ///
    /// - **Windows:** The executable retains its `.exe` extension (if present), and no permission
    ///   setting is required.
    /// - **Linux (Unix):** Sets executable permissions explicitly.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The current executable cannot be determined.
    /// - The installation directory cannot be created.
    /// - The file copy operation fails.
    /// - (Unix only) Permissions cannot be set.
    pub fn build(&self) -> std::io::Result<()> {
        let current_exe = env::current_exe()?;
        trace!("Current executable path: {:?}", current_exe);
        trace!("path: {}", self.path);

        fs::create_dir_all(&self.install_dir)?;

        let dest_path = self.install_dir.join(
            current_exe
                .file_name()
                .ok_or(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Executable name not found",
                ))?,
        );
        fs::copy(&current_exe, &dest_path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&dest_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&dest_path, perms)?;
        }

        trace!("Executable installed to: {:?}", dest_path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    #[cfg(unix)]
    fn test_new_local_cli_builder_system_wide() {
        let cli_builder = LocalCLIBuilder::new("/some/path".to_string(), true);
        assert_eq!(cli_builder.path, "/some/path");
        assert!(cli_builder.system_wide);
        assert_eq!(cli_builder.install_dir, PathBuf::from("/usr/local/bin"));
    }

    #[test]
    #[cfg(unix)]
    fn test_new_local_cli_builder_user_local() {
        let cli_builder = LocalCLIBuilder::new("/some/path".to_string(), false);
        assert_eq!(cli_builder.path, "/some/path");
        assert!(!cli_builder.system_wide);
        assert_eq!(cli_builder.install_dir, dirs::home_dir().expect("iternal error").join(".local/bin"));
    }

    // #[test] //awkward test messing with real folders.
    // fn test_with_custom_location_not_in_path() {
    //     let temp_dir = tempdir().expect("iternal error");
    //     let custom_path = temp_dir.path().join("custom_bin_not_in_path");
    //
    //     let cli_builder = LocalCLIBuilder::new("/some/path".to_string(), true)
    //         .with_custom_location(custom_path.clone());
    //
    //     assert_ne!(cli_builder.install_dir, custom_path);
    //     assert_eq!(cli_builder.install_dir, PathBuf::from("/usr/local/bin"));
    // }

    #[test]
    #[cfg(unix)]
    fn test_build_creates_executable_in_install_dir() {
        let temp_dir = tempdir().expect("iternal error");
        let custom_install_dir = temp_dir.path().join("bin");
        fs::create_dir_all(&custom_install_dir).expect("iternal error");

        let cli_builder = LocalCLIBuilder::new("/some/path".to_string(), false)
            .with_custom_location(custom_install_dir.clone());

        let exe_path = custom_install_dir.join("test_exe");
        fs::File::create(&exe_path).expect("iternal error");

        let result = cli_builder.build();
        assert!(result.is_ok());

        let installed_exe = custom_install_dir.join("test_exe");
        assert!(installed_exe.exists());

        let _metadata = fs::metadata(&installed_exe).expect("iternal error");

    }

    // #[test]
    // fn test_build_invalid_path() {
    //     let cli_builder = LocalCLIBuilder::new("/invalid/path".to_string(), false);
    //
    //     let result = cli_builder.build();
    //     assert!(result.is_err());
    // }

    // #[test]
    // fn test_build_install_dir_creation_failure() {
    //     let cli_builder = LocalCLIBuilder::new("/some/path".to_string(), false)
    //         .with_custom_location(PathBuf::from("/root/protected_dir"));
    //
    //     let result = cli_builder.build();
    //     error!("{:?}", result);
    //    // assert!(result.is_err());
    // }

    #[cfg(not(windows))]
    #[test]
    fn test_default_installation_directory_system_wide() {
        let cli_builder = LocalCLIBuilder::new("/some/path".to_string(), true);
        assert_eq!(cli_builder.install_dir, PathBuf::from("/usr/local/bin"));
    }

    #[test]
    #[cfg(unix)]
    fn test_default_installation_directory_user_local() {
        let cli_builder = LocalCLIBuilder::new("/some/path".to_string(), false);
        let expected_dir = dirs::home_dir().expect("iternal error").join(".local/bin");
        assert_eq!(cli_builder.install_dir, expected_dir);
    }
}

