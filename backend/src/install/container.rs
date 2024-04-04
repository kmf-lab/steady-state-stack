

// builds a dockerfile where we can choose to build the app for ARM or INTEL
// we can choose alpine for small footprint or other choices like debian for stability.

/*
# Use the official Rust image as the build stage
FROM --platform=linux/amd64 rust:latest as build-amd64
# Your build commands here
WORKDIR /usr/src/myapp
COPY . .
RUN cargo build --release

# Use the official Rust image as the build stage for ARM
FROM --platform=linux/arm/v7 rust:latest as build-arm
# Your build commands here
WORKDIR /usr/src/myapp
COPY . .
RUN cargo build --release

# Final stage, choose the appropriate build image
FROM debian:buster
COPY --from=build-amd64 /usr/src/myapp/target/release/myapp /usr/local/bin/myapp
# or use build-arm for ARM architecture
# COPY --from=build-arm /usr/src/myapp/target/release/myapp /usr/local/bin/myapp
*/
//docker buildx build --platform linux/arm/v7 -t your-image-name:tag .


use log::*;

#[derive(Clone)]
pub struct ContainerBuilder {
    name: String

}

impl ContainerBuilder {

    pub fn new(name: String) -> Self {
        ContainerBuilder {
            name,
        }
    }

    pub fn build(&self) {
          info!("name: {}", self.name);
    }

}


