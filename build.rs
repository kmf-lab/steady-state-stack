use std::process::{Command, Stdio};
use std::{env, fs};
use std::fs::File;
use std::io::Write;
use std::path::Path;

fn main() {
    // Define the path to the file relative to the project root

    gzip_and_base64_encode("static/telemetry/viz-lite.js");
    gzip_and_base64_encode("static/telemetry/dot-viewer.js");
    gzip_and_base64_encode("static/telemetry/dot-viewer.css");
    gzip_and_base64_encode("static/telemetry/index.html");
    gzip_and_base64_encode("static/telemetry/webworker.js");

    base64_encode("static/telemetry/images/spinner.gif");

}


fn base64_encode(file_path: &str) {
    let project_root = env::var("CARGO_MANIFEST_DIR").unwrap_or(".".to_string());
    let output_name = format!("{}.b64", file_path);
    let target_file = &Path::new(&project_root).join("target").join(output_name);
    // Check if the output file already exists
    if Path::new(&target_file).exists() {
        println!("{:?} already exists, skipping", target_file);
        return;
    }
    if let Some(parent_dir) = target_file.parent() {
        fs::create_dir_all(parent_dir).expect("Failed to create output directory");
    }

    // Open the output file for writing
    let mut output_file = File::create(target_file).expect("Failed to create output file");

    // Run the base64 command
    let base64_output = Command::new("base64")
        .arg("-w")
        .arg("0")
        .arg(file_path)
        .output()
        .expect("Failed to execute base64 command");

    // Write the base64 encoded data to the output file
    output_file.write_all(&base64_output.stdout).expect("Failed to write to output file");

    println!("Processed and saved to {:?}", target_file);
}


fn gzip_and_base64_encode(file_path: &str) {
    let project_root = env::var("CARGO_MANIFEST_DIR").unwrap_or(".".to_string());
    let output_name = format!("{}.gz.b64", file_path);
    let target_file = &Path::new(&project_root).join("target").join(output_name);
    // Check if the output file already exists
    if Path::new(&target_file).exists() {
        println!("{:?} already exists, skipping", target_file);
        return;
    }
    if let Some(parent_dir) = target_file.parent() {
        fs::create_dir_all(parent_dir).expect("Failed to create output directory");
    }
    // Open the output file for writing
    let mut output_file = File::create(target_file).expect("Failed to create output file");

    // Run the gzip command and pipe its output to the base64 command
    let gzip_output = Command::new("gzip")
        .arg("-c")
        .arg(file_path)
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to start gzip process")
        .stdout
        .expect("Failed to open gzip stdout");

    let base64_output = Command::new("base64")
        .arg("-w")
        .arg("0")
        .stdin(gzip_output)
        .output()
        .expect("Failed to execute base64 command");

    // Write the base64 encoded data to the output file
    output_file.write_all(&base64_output.stdout).expect("Failed to write to output file");

    println!("Processed and saved to {:?}", target_file);

}
