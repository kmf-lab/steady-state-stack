use std::process::{Command, Stdio};
use std::env;
use std::fs::File;
use std::io::Write;
use std::path::Path;

fn main() {
    // Define the path to the file relative to the project root
    let project_root = env::var("CARGO_MANIFEST_DIR").unwrap_or(".".to_string());

    gzip_and_base64_encode(&Path::new(&project_root).join("static/telemetry/viz-lite.js"));
    gzip_and_base64_encode(&Path::new(&project_root).join("static/telemetry/dot-viewer.js"));
    gzip_and_base64_encode(&Path::new(&project_root).join("static/telemetry/dot-viewer.css"));
    gzip_and_base64_encode(&Path::new(&project_root).join("static/telemetry/index.html"));
    gzip_and_base64_encode(&Path::new(&project_root).join("static/telemetry/webworker.js"));

    base64_encode(&Path::new(&project_root).join("static/telemetry/images/spinner.gif"));


}


fn base64_encode(file_path: &Path) {
    // Define the output file name
    let output_file_name = format!("{}.b64", file_path.to_str().unwrap_or("UNKNOWN"));

    // Check if the output file already exists
    if Path::new(&output_file_name).exists() {
        println!("{} already exists, skipping", output_file_name);
        return;
    }

    // Open the output file for writing
    let mut output_file = File::create(&output_file_name).expect("Failed to create output file");

    // Run the base64 command
    let base64_output = Command::new("base64")
        .arg("-w")
        .arg("0")
        .arg(file_path)
        .output()
        .expect("Failed to execute base64 command");

    // Write the base64 encoded data to the output file
    output_file.write_all(&base64_output.stdout).expect("Failed to write to output file");

    println!("Processed and saved to {}", output_file_name);
}


fn gzip_and_base64_encode(file_path: &Path) {
    // Define the output file name
    let output_file_name = format!("{}.gz.b64", file_path.to_str().unwrap_or("UNKNOWN"));

    // Check if the output file already exists
    if Path::new(&output_file_name).exists() {
        println!("{} already exists, skipping", output_file_name);
        return;
    }

    // Open the output file for writing
    let mut output_file = File::create(&output_file_name).expect("Failed to create output file");

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

    println!("Processed and saved to {}", output_file_name);

}
