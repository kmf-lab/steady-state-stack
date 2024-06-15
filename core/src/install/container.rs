//! This module provides functionality to build Docker containers for the SteadyState project.
//! It allows configuring various parameters such as base image, runtime image, Dockerfile path,
//! target architecture, and destination repository. It supports both standard Docker builds and Docker Buildx.

//TODO: this module is probably a mistake instead we should have the code generate make the dockerfile.

use std::fs;
use std::path::Path;
use log::*;
use std::process::Command;

/// A builder for creating Docker containers.
#[derive(Clone)]
pub struct ContainerBuilder {
    base_image: String,
    runtime_image: String,
    dockerfile_path: String,
    target_arch: String,
    destination_repo: Option<String>,
    with_build: bool,
    use_buildx: bool,  // Flag to indicate use of Docker Buildx
}

impl ContainerBuilder {
    /// Creates a new `ContainerBuilder`.
    ///
    /// # Arguments
    ///
    /// * `_project_path` - The path to the project.
    ///
    /// # Returns
    ///
    /// A new `ContainerBuilder` instance.
    pub fn new(_project_path: String) -> Self {
        ContainerBuilder {
            base_image: "rust:latest".to_string(),
            runtime_image: "debian:stable-slim".to_string(),
            dockerfile_path: "Dockerfile".to_string(),
            target_arch: "amd64".to_string(),
            destination_repo: None,
            with_build: false,
            use_buildx: false,  // Default to not using Buildx
        }
    }

    /// Sets the use of Docker Buildx.
    ///
    /// # Arguments
    ///
    /// * `use_buildx` - A boolean indicating if Docker Buildx should be used.
    ///
    /// # Returns
    ///
    /// The `ContainerBuilder` instance with the updated use_buildx flag.
    pub fn with_buildx(&mut self, use_buildx: bool) -> &mut Self {
        self.use_buildx = use_buildx;
        self
    }

    /// Sets the base image for the Docker container.
    ///
    /// # Arguments
    ///
    /// * `base_image` - The base image.
    ///
    /// # Returns
    ///
    /// The `ContainerBuilder` instance with the updated base image.
    pub fn with_base_image(&mut self, base_image: String) -> &mut Self {
        self.base_image = base_image;
        self
    }

    /// Sets the runtime image for the Docker container.
    ///
    /// # Arguments
    ///
    /// * `runtime_image` - The runtime image.
    ///
    /// # Returns
    ///
    /// The `ContainerBuilder` instance with the updated runtime image.
    pub fn with_runtime_image(&mut self, runtime_image: String) -> &mut Self {
        self.runtime_image = runtime_image;
        self
    }

    /// Sets the Dockerfile path for the Docker container.
    ///
    /// # Arguments
    ///
    /// * `dockerfile_path` - The Dockerfile path.
    ///
    /// # Returns
    ///
    /// The `ContainerBuilder` instance with the updated Dockerfile path.
    pub fn with_dockerfile_path(&mut self, dockerfile_path: String) -> &mut Self {
        self.dockerfile_path = dockerfile_path;
        self
    }

    /// Sets the target architecture for the Docker container.
    ///
    /// # Arguments
    ///
    /// * `target_arch` - The target architecture.
    ///
    /// # Returns
    ///
    /// The `ContainerBuilder` instance with the updated target architecture.
    pub fn with_target_arch(&mut self, target_arch: String) -> &mut Self {
        self.target_arch = target_arch;
        self
    }

    /// Sets the destination repository for the Docker container.
    ///
    /// # Arguments
    ///
    /// * `destination_repo` - The destination repository.
    ///
    /// # Returns
    ///
    /// The `ContainerBuilder` instance with the updated destination repository.
    pub fn with_destination_repo(&mut self, destination_repo: String) -> &mut Self {
        self.destination_repo = Some(destination_repo);
        self
    }

    /// Sets whether to build the Docker container.
    ///
    /// # Arguments
    ///
    /// * `with_build` - A boolean indicating if the container should be built.
    ///
    /// # Returns
    ///
    /// The `ContainerBuilder` instance with the updated build flag.
    pub fn with_build(&mut self, with_build: bool) -> &mut Self {
        self.with_build = with_build;
        self
    }

    /// Sets the runtime image to a reliable Debian image.
    ///
    /// # Returns
    ///
    /// The `ContainerBuilder` instance with the updated runtime image.
    pub fn with_reliable(&mut self) -> &mut Self {
        self.runtime_image = "debian:stable-slim".to_string();
        self
    }

    /// Sets the runtime image to a tiny Alpine image.
    ///
    /// # Returns
    ///
    /// The `ContainerBuilder` instance with the updated runtime image.
    pub fn with_tiny(&mut self) -> &mut Self {
        self.runtime_image = "alpine".to_string();
        self
    }

    /// Builds the Docker container.
    ///
    /// # Arguments
    ///
    /// * `project_path` - The path to the project.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub fn build(&self, project_path: String) -> Result<(), Box<dyn std::error::Error>> {
        let project_name = Path::new(&project_path)
            .file_name()
            .ok_or("Invalid project path")?
            .to_str()
            .ok_or("Invalid UTF-8 in project path")?;

        let dockerfile_content = format!(r#"
FROM {base_image} as builder
WORKDIR /app
COPY . .
RUN apt-get update && apt-get install -y build-essential
RUN cargo build --release --target {target_arch}

FROM {runtime_image}
COPY --from=builder /app/target/{target_arch}/release/{project_name} /usr/local/bin/
ENTRYPOINT ["/usr/local/bin/{project_name}"]
"#,
                                         base_image = self.base_image,
                                         runtime_image = self.runtime_image,
                                         target_arch = self.target_arch,
                                         project_name = project_name,
        );

        fs::write(&self.dockerfile_path, dockerfile_content)?;
        info!("Dockerfile created at: {}", self.dockerfile_path);

        if self.with_build {
            let mut build_command = Command::new("docker");

            if self.use_buildx {
                build_command.arg("buildx").arg("build").arg("--platform").arg(&format!("linux/{}", self.target_arch));
            } else {
                build_command.arg("build");
            }

            build_command.arg("-t");

            if let Some(repo) = &self.destination_repo {
                build_command.arg(repo);
            } else {
                build_command.arg(project_name);
            }

            build_command.arg(".");
            let output = build_command.output()?;

            if output.status.success() {
                info!("Docker image built successfully");
            } else {
                error!("Failed to build Docker image");
                error!("stdout: {}", String::from_utf8_lossy(&output.stdout));
                error!("stderr: {}", String::from_utf8_lossy(&output.stderr));
                return Err("Docker build failed".into());
            }
        }

        Ok(())
    }
}
