use std::process::{Command, Stdio};
use std::env;
use std::fs::File;
use std::path::{Path, PathBuf};

fn main() {
    // Define the path to the file relative to the project root
    let project_root = env::var("CARGO_MANIFEST_DIR").unwrap();

    gzip_me(&Path::new(&project_root).join("static/telemetry/viz-lite.js"));
    gzip_me(&Path::new(&project_root).join("static/telemetry/dot-viewer.js"));
    gzip_me(&Path::new(&project_root).join("static/telemetry/dot-viewer.css"));
    gzip_me(&Path::new(&project_root).join("static/telemetry/index.html"));
    gzip_me(&Path::new(&project_root).join("static/telemetry/webworker.js"));
}

fn gzip_me(file_path: &PathBuf) {
    // Create the output file path with .gz extension
    let output_path = file_path.with_extension("gz");

    // Open the output file for writing
    let output_file = File::create(&output_path).expect("Failed to create output file");

    // Run the gzip command with the output file as stdout
    let status = Command::new("gzip")
        .arg("-c")
        .arg(file_path)
        .stdout(Stdio::from(output_file))
        .status()
        .expect("Failed to execute gzip command");

    println!("gzip status: {}", status);
}
