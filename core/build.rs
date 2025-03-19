//! Build process for the steady_state crate. Ensures telemetry web files are prepared and placed
//! in the target directory before building. These files can be included in the binary using the
//! `include_str!` macro when specific features are enabled.
//!
//! ### Features:
//! - `telemetry_server_builtin`: Embeds the telemetry server in the binary (great for offline use).
//! - `telemetry_server_cdn`: Uses a CDN for telemetry resources (reduces binary size, requires internet).
//! - `prometheus_metrics`: Includes a Prometheus server for metrics scraping.
//! - `telemetry_history`: Generates telemetry history files for playback.
//! - `proactor_nuclei`: Uses the nuclei-based proactor with io_uring (default async runtime).
//! - `proactor_tokio`: Uses the Tokio-based proactor instead.

use std::env;
use std::fs::{self, File};
use std::io::{self};
use std::path::{Path, PathBuf};
use askama::Template;
use flate2::write::GzEncoder;
use flate2::Compression;

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

/// Version of viz.js to use for telemetry visualization.
const VIZ_VERSION: &str = "1.8.2";

/// Determines if viz-lite.js should be included locally (true if telemetry_server_builtin is enabled).
#[cfg(not(feature = "telemetry_server_builtin"))]
const USE_INTERNAL_VIZ: bool = false;
#[cfg(feature = "telemetry_server_builtin")]
const USE_INTERNAL_VIZ: bool = true;

/// Indicates if telemetry services are active (true if either telemetry_server_cdn or telemetry_server_builtin is enabled).
#[cfg(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))]
const TELEMETRY_SERVICE: bool = true;
#[cfg(not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const TELEMETRY_SERVICE: bool = false;

fn main() {
    // Print sponsorship messages to encourage community support
    println!("cargo:warning=########### Community support needed ###########################");
    println!("cargo:warning=Please Sponsor Steady_State: https://github.com/sponsors/kmf-lab");
    println!("cargo:warning=################################################################");

    // Determine the base target directory for output files
    let base_target_path = env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target"));

    if TELEMETRY_SERVICE && USE_INTERNAL_VIZ {
        // Process viz-lite.js if telemetry is enabled with internal visualization
        // Check for new versions at: https://github.com/mdaines/viz-js/releases
        gzip_encode_web_resource(
            &base_target_path,
            "static/telemetry/viz-lite.js",
            &format!("https://unpkg.com/viz.js@{}/viz-lite.js", VIZ_VERSION),
        );
    }

    if TELEMETRY_SERVICE {
        // Handle telemetry-related files when the service is enabled
        let cdn_source = format!("https://unpkg.com/viz.js@{}/viz-lite.js", VIZ_VERSION);
        let source = if USE_INTERNAL_VIZ {
            "viz-lite.js" // Local file reference when embedded in the binary
        } else {
            &cdn_source // CDN URL when using external resources
        };

        let folder_base = PathBuf::from("static/telemetry/");

        // Generate and write index.html from the template
        let index_content = IndexTemplate { script_source: source }
            .render()
            .expect("Failed to render index.html template");
        fs::write(folder_base.join("index.html"), index_content)
            .expect("Failed to write index.html");
        // Always encode index.html since it’s freshly generated
        gzip_encode(&base_target_path, "static/telemetry/index.html", false);

        // Generate and write webworker.js from the template
        let webworker_content = WebWorkerTemplate { script_source: source }
            .render()
            .expect("Failed to render webworker.js template");
        fs::write(folder_base.join("webworker.js"), webworker_content)
            .expect("Failed to write webworker.js");
        // Always encode webworker.js since it’s freshly generated
        gzip_encode(&base_target_path, "static/telemetry/webworker.js", false);

        // Encode static files, skipping if their gzipped versions already exist
        gzip_encode(&base_target_path, "static/telemetry/dot-viewer.js", true);
        gzip_encode(&base_target_path, "static/telemetry/dot-viewer.css", true);

        // Copy spinner.gif to the target directory without compression
        let file_path = "static/telemetry/images/spinner.gif";
        simple_copy(Path::new(file_path), &base_target_path.join(file_path));
    }
}

/// Copies a file from source to target, creating directories as needed.
/// Returns true if the file was skipped (already exists), false if copied.
fn simple_copy(source_file: &Path, target_file: &PathBuf) -> bool {
    if target_file.exists() {
        println!("cargo:warning={:?} already exists, skipping copy", target_file);
        return true;
    }

    // Ensure the parent directory exists
    if let Some(parent_dir) = target_file.parent() {
        fs::create_dir_all(parent_dir).expect("Failed to create target directory");
    }

    // Stream the file content from source to target
    let mut source = File::open(source_file).expect("Failed to open source file");
    let mut target = File::create(target_file).expect("Failed to create target file");
    io::copy(&mut source, &mut target).expect("Failed to copy file content");

    println!("cargo:warning=Copied {:?} to {:?}", source_file, target_file);
    false
}

/// Downloads and gzip-encodes viz-lite.js if its compressed version is missing.
/// Cleans up the temporary downloaded file afterward.
fn gzip_encode_web_resource(target: &Path, file_path: &str, get_url: &str) {
    let output_name = format!("{}.gz", file_path);
    let target_file = target.join(&output_name);

    if !target_file.exists() {
        // Download the file since its gzipped version doesn’t exist
        let mut response = isahc::get(get_url).expect("Failed to download viz-lite.js");
        let mut file = File::create(file_path).expect("Failed to create temporary file");
        io::copy(&mut response.body_mut(), &mut file)
            .expect("Failed to write downloaded content");
    }

    // Encode the file (will proceed since we just checked the .gz file’s absence)
    gzip_encode(target, file_path, true);

    // Clean up the temporary file if it exists
    if Path::new(file_path).exists() {
        fs::remove_file(file_path).expect("Failed to remove temporary file");
    }
}

/// Gzip-encodes a file and saves it to the target directory.
/// Skips encoding if `skip_if_exists` is true and the output file already exists.
fn gzip_encode(target: &Path, file_path: &str, skip_if_exists: bool) {
    let output_name = format!("{}.gz", file_path);
    let target_file = target.join(&output_name);

    // Create parent directories if they don’t exist
    if let Some(parent_dir) = target_file.parent() {
        fs::create_dir_all(parent_dir).expect("Failed to create output directory");
    }

    if skip_if_exists && target_file.exists() {
        println!("cargo:warning={:?} already exists, skipping compression", target_file);
        return;
    }

    // Open the source file and create the gzipped output file
    let mut input = File::open(file_path).expect("Failed to open input file");
    let output = File::create(&target_file).expect("Failed to create output file");

    // Compress the file using flate2’s GzEncoder
    let mut encoder = GzEncoder::new(output, Compression::default());
    io::copy(&mut input, &mut encoder).expect("Failed to compress file");
    encoder.finish().expect("Failed to finalize compression");

    println!("cargo:warning=Compressed {:?} to {:?}", file_path, target_file);
}