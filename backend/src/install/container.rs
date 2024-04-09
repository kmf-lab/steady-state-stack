use std::fs;
use std::path::Path;
use log::*;
use std::process::Command;


// TODO: this location is probably a mistake
//     the code generator should generate this docker file for your project if desired
//     on some box with docker you should be able to git clone docker build


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
    pub fn new(project_path: String) -> Self {
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

    pub fn with_buildx(&mut self, use_buildx: bool) -> &mut Self {
        self.use_buildx = use_buildx;
        self
    }

    pub fn with_base_image(&mut self, base_image: String) -> &mut Self {
        self.base_image = base_image;
        self
    }

    pub fn with_runtime_image(&mut self, runtime_image: String) -> &mut Self {
        self.runtime_image = runtime_image;
        self
    }

    pub fn with_dockerfile_path(&mut self, dockerfile_path: String) -> &mut Self {
        self.dockerfile_path = dockerfile_path;
        self
    }

    pub fn with_target_arch(&mut self, target_arch: String) -> &mut Self {
        self.target_arch = target_arch;
        self
    }

    pub fn with_destination_repo(&mut self, destination_repo: String) -> &mut Self {
        self.destination_repo = Some(destination_repo);
        self
    }

    pub fn with_build(&mut self, with_build: bool) -> &mut Self {
        self.with_build = with_build;
        self
    }

    pub fn with_reliable(&mut self) -> &mut Self {
        self.runtime_image = "debian:stable-slim".to_string();
        self
    }

    pub fn with_tiny(&mut self) -> &mut Self {
        self.runtime_image = "alpine".to_string();
        self
    }

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
