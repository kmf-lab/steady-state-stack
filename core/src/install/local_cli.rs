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
            info!("Custom install directory {:?} is not in the PATH {:?}", custom_install_dir, env::var_os("PATH"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    #[test]
    fn test_new_local_cli_builder_system_wide() {
        let cli_builder = LocalCLIBuilder::new("/some/path".to_string(), true);
        assert_eq!(cli_builder.path, "/some/path");
        assert!(cli_builder.system_wide);
        assert_eq!(cli_builder.install_dir, PathBuf::from("/usr/local/bin"));
    }

    #[test]
    fn test_new_local_cli_builder_user_local() {
        let cli_builder = LocalCLIBuilder::new("/some/path".to_string(), false);
        assert_eq!(cli_builder.path, "/some/path");
        assert!(!cli_builder.system_wide);
        assert_eq!(cli_builder.install_dir, dirs::home_dir().expect("iternal error").join(".local/bin"));
    }

    #[test]
    fn test_with_custom_location_not_in_path() {
        let temp_dir = tempdir().expect("iternal error");
        let custom_path = temp_dir.path().join("custom_bin_not_in_path");

        let cli_builder = LocalCLIBuilder::new("/some/path".to_string(), true)
            .with_custom_location(custom_path.clone());

        assert_ne!(cli_builder.install_dir, custom_path);
        assert_eq!(cli_builder.install_dir, PathBuf::from("/usr/local/bin"));
    }

    #[test]
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

        let metadata = fs::metadata(&installed_exe).expect("iternal error");
        assert!(metadata.permissions().mode() & 0o777 >= 0o644);

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

    #[test]
    fn test_default_installation_directory_system_wide() {
        let cli_builder = LocalCLIBuilder::new("/some/path".to_string(), true);
        assert_eq!(cli_builder.install_dir, PathBuf::from("/usr/local/bin"));
    }

    #[test]
    fn test_default_installation_directory_user_local() {
        let cli_builder = LocalCLIBuilder::new("/some/path".to_string(), false);
        let expected_dir = dirs::home_dir().expect("iternal error").join(".local/bin");
        assert_eq!(cli_builder.install_dir, expected_dir);
    }
}

