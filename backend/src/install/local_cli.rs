use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use log::*;
use dirs; // Ensure `dirs` crate is added to your `Cargo.toml`

#[derive(Clone)]
pub struct LocalCLIBuilder {
    pub(crate) path: String,
    pub(crate) system_wide: bool,
    pub(crate) install_dir: PathBuf,
}

impl LocalCLIBuilder {
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

    /// only use this if you know you need it for some custom distro
    pub fn with_custom_location(mut self, custom_install_dir: PathBuf) -> Self {
        let in_path = env::var_os("PATH")
            .map_or(false, |paths| paths.to_string_lossy().contains(&*custom_install_dir.to_string_lossy()));


        if in_path {
            self.install_dir = custom_install_dir;
        } else {
            warn!("Custom install directory is not in the PATH");
        }
        self
    }

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
