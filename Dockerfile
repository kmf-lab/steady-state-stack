# docker build -t steady .


# Start from the official Rust image to ensure we have the latest version of Rust and Cargo
FROM rust:1.82 as builder

# Set the working directory inside the container
WORKDIR /usr/src/myapp

# Copy the actual source code of the Rust project into the Docker image
COPY . .
RUN cargo fetch
# above this point we hope to have cached all our crates

RUN cargo build
RUN cargo test
RUN cargo test --example simple_widgets 
RUN cargo test --example large_scale
 


