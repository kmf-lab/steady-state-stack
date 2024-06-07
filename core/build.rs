use std::process::{Command, Stdio};
use std::{env, fs};
use std::fs::{copy, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use askama::Template;
use isahc::prelude::*;
use isahc::{Body, Error, Response};

#[derive(Template)]
#[template(path = "index.html.txt")]
pub(crate) struct IndexTemplate<'a> {
    pub(crate) script_source: &'a str,
}

#[derive(Template)]
#[template(path = "webworker.js.txt")]
pub(crate) struct WebWorkerTemplate<'a> {
    pub(crate) script_source: &'a str,
}

const viz_version:&str = "1.8.2";

#[cfg(not(feature = "telemetry_server_builtin"))]
const use_internal_viz:bool = false;
#[cfg(feature = "telemetry_server_builtin")]
const use_internal_viz:bool = true;

#[cfg(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))]
const telemetry_service: bool = true;
#[cfg(not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const telemetry_service: bool = false;
fn main() {

    let base_target_path: PathBuf = env::var("CARGO_TARGET_DIR").map_or_else(
        |_| PathBuf::from("target"), // Fallback to local target directory if CARGO_TARGET_DIR is not set
        PathBuf::from,
    );

    if telemetry_service && use_internal_viz {
        // To check on new versions go here:    https://github.com/mdaines/viz-js/releases
        gzip_and_base64_encode_web_resource(&base_target_path
                                            ,"static/telemetry/viz-lite.js"
                                            ,&format!("https://unpkg.com/viz.js@{}/viz-lite.js",viz_version));
    }

    if telemetry_service {

        let cdn_source = format!("https://unpkg.com/viz.js@{}/viz-lite.js",viz_version);
        let source = if use_internal_viz {
            "viz-lite.js" //only when we ship this inside the binary
        } else {
            &cdn_source // using cdn
        };
        let folder_base = PathBuf::from("static/telemetry/");
        if let Err(e) = fs::write(folder_base.join("index.html"), IndexTemplate { script_source: &source }.render().expect("unable to render")) {
            panic!("Error writing index.html {:?}", e);
        };
        if let Err(e) = fs::write(folder_base.join("webworker.js"), WebWorkerTemplate { script_source: &source }.render().expect("unable to render")) {
            panic!("Error writing webworker.js {:?}", e);
        };

        //simple encode and copy to the destination when telemetry is enabled
        gzip_and_base64_encode(&base_target_path, "static/telemetry/viz-lite.js");
        gzip_and_base64_encode(&base_target_path, "static/telemetry/dot-viewer.js");
        gzip_and_base64_encode(&base_target_path, "static/telemetry/dot-viewer.css");
        base64_encode(&base_target_path, "static/telemetry/images/spinner.gif");
    }
}


fn base64_encode(target: &Path, file_path: &str) {

    let output_name = format!("{}.b64", file_path);
    let target_file = target.join(output_name);
    // Check if the output file already exists
    if Path::new(&target_file).exists() {
        println!("{:?} already exists, skipping", target_file);
        return;
    }
    if let Some(parent_dir) = target_file.parent() {
        fs::create_dir_all(parent_dir).expect("Failed to create output directory");
    }

    // Open the output file for writing
    let mut output_file = File::create(target_file.clone()).expect("Failed to create output file");

    // Run the base64 command
    let base64_output = Command::new("base64")
        .arg("-w")
        .arg("0")
        .arg(file_path)
        .output()
        .expect("Failed to execute base64 command");

    // Write the base64 encoded data to the output file
    output_file.write_all(&base64_output.stdout).expect("Failed to write to output file");

    println!("Processed and saved to {:?}", &target_file);
}

fn gzip_and_base64_encode_web_resource(target: &Path, file_path: &str, get_url: &str) {

    let output_name = format!("{}.gz.b64", file_path);
    let target_file = target.join(output_name);
    // Check if the output file already exists
    if !Path::new(&target_file).exists() || !Path::new(&file_path).exists()  {
        //download the file since our copy or the gziped version does not exist
        //and write it to the target file
        let mut response = isahc::get(get_url);
        match response {
            Ok(mut response) => {
                let mut content = Vec::new();
                response.body_mut().read_to_end(&mut content).expect("Failed to read response");
                let mut file = File::create(&file_path).expect("Failed to create file");
                file.write_all(&content).expect("Failed to write to file");
            }
            Err(e) => {
                panic!("Error downloading {} {}",get_url, e);
            }
        }
    }
    gzip_and_base64_encode(&target, file_path);

}
fn gzip_and_base64_encode(target: &Path, file_path: &str) {

    let output_name = format!("{}.gz.b64", file_path);
    let target_file = target.join(output_name);
    // Check if the output file already exists
    if Path::new(&target_file).exists() {
        println!("{:?} already exists, skipping", target_file);
        return;
    }
    if let Some(parent_dir) = target_file.parent() {
        fs::create_dir_all(parent_dir).expect("Failed to create output directory");
    }
    // Open the output file for writing
    let mut output_file = File::create(target_file.clone()).expect("Failed to create output file");

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

    println!("Processed and saved to {:?}", &target_file);

}
