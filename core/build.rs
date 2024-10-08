//! Build process for steady-state. Ensures we have the telemetry web files in place before building.
//! The macro `include_str!` is used to include the contents of the files in the binary if enabled.

/// Features:
/// - telemetry_server_builtin: Include the telemetry server in the binary
///                   Great for offline or internet isolated projects
/// - telemetry_server_cdn: Use the telemetry server from a CDN
///                   Good for smaller binary and projects which have internet access
/// - prometheus_metrics: Include the prometheus server to be scraped in the binary
/// - telemetry_history: Generate history files of the telemetry for playback later
/// - proactor_nuclei: Use the nuclei based proactor io_uring async runtime (default)
/// - praactor_tokio: Use the tokio based proactor io_uring async runtime

use std::process::{Command, Stdio};
use std::{env, fs};
use std::fs::{File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use askama::Template;


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

const VIZ_VERSION:&str = "1.8.2";

#[cfg(not(feature = "telemetry_server_builtin"))]
const USE_INTERNAL_VIZ:bool = false;
#[cfg(feature = "telemetry_server_builtin")]
const USE_INTERNAL_VIZ:bool = true;

#[cfg(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))]
const TELEMETRY_SERVICE: bool = true;
#[cfg(not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const TELEMETRY_SERVICE: bool = false;
fn main() {
    //all text here is blocked except for the warning messages starting with cargo:warning=
    println!("cargo:warning=########### Community support needed ###########################");
    println!("cargo:warning=Please Sponsor Steady_State: https://github.com/sponsors/kmf-lab");
    println!("cargo:warning=################################################################");
  
    let base_target_path: PathBuf = env::var("CARGO_TARGET_DIR").map_or_else(
        |_| PathBuf::from("target"), // Fallback to local target directory if CARGO_TARGET_DIR is not set
        PathBuf::from,
    );

    if TELEMETRY_SERVICE && USE_INTERNAL_VIZ {
        // To check on new versions go here:    https://github.com/mdaines/viz-js/releases
        gzip_and_base64_encode_web_resource(&base_target_path
                                            ,"static/telemetry/viz-lite.js"
                                            ,&format!("https://unpkg.com/viz.js@{}/viz-lite.js", VIZ_VERSION));
    }

    if TELEMETRY_SERVICE {

        let cdn_source = format!("https://unpkg.com/viz.js@{}/viz-lite.js", VIZ_VERSION);
        let source = if USE_INTERNAL_VIZ {
            "viz-lite.js" //only when we ship this inside the binary
        } else {
            &cdn_source // using cdn
        };
        let folder_base = PathBuf::from("static/telemetry/");
        if let Err(e) = fs::write(folder_base.join("index.html"), IndexTemplate { script_source: source }.render().expect("unable to render")) {
            panic!("Error writing index.html {:?}", e);
        };
        gzip_and_base64_encode(&base_target_path, "static/telemetry/index.html",false);

        if let Err(e) = fs::write(folder_base.join("webworker.js"), WebWorkerTemplate { script_source: source }.render().expect("unable to render")) {
            panic!("Error writing webworker.js {:?}", e);
        };
        gzip_and_base64_encode(&base_target_path, "static/telemetry/webworker.js",false);

        //simple encode and copy to the destination when telemetry is enabled
        gzip_and_base64_encode(&base_target_path, "static/telemetry/dot-viewer.js",true);
        gzip_and_base64_encode(&base_target_path, "static/telemetry/dot-viewer.css",true);
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

    // Run the base64 command, strange but the base64 crate is always broken
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
    if !Path::new(&target_file).exists() {
        //download the file since our copy or the gziped version does not exist
        //and write it to the target file
        let response = isahc::get(get_url);
        match response {
            Ok(mut response) => {
                let mut content = Vec::new();
                response.body_mut().read_to_end(&mut content).expect("Failed to read response");
                let mut file = File::create(file_path).expect("Failed to create file");
                file.write_all(&content).expect("Failed to write to file");
            }
            Err(e) => {
                panic!("Error downloading {} {}",get_url, e);
            }
        }
    }
    gzip_and_base64_encode(target, file_path,true);
    //cleanup the downloaded file
    //only remove the file if it exists
    if Path::new(&file_path).exists() {
        fs::remove_file(file_path).expect("Failed to remove downloaded file");
    }
}
fn gzip_and_base64_encode(target: &Path, file_path: &str, skip_if_exists:bool) {

    let output_name = format!("{}.gz.b64", file_path);
    let target_file = target.join(output_name);
    // Check if the output file already exists
    if skip_if_exists && Path::new(&target_file).exists() {
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

    // Run the base64 command, strange but the base64 crate is always broken
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
