//! Build script for the `steady_state` crate: prepares telemetry web assets for **compile-time**
//! embedding via `include_bytes!` / `include_str!`.
//!
//! ## Critical: `OUT_DIR` vs `CARGO_TARGET_DIR` (do not regress)
//!
//! Any blob that `src/telemetry/metrics_server.rs` embeds with `include_bytes!(concat!(env!("OUT_DIR"), "/…"))`
//! **must be written into Cargo’s `OUT_DIR`** by this script. Cargo sets `OUT_DIR` to a directory
//! private to this package’s build; `rustc` resolves `env!("OUT_DIR")` to that **same** path when
//! compiling the library, so the build script and the embed macros always agree.
//!
//! **`CARGO_TARGET_DIR` must not be the only place those embeddable files live.** It is the shared
//! artifact directory for the workspace or the root package being built. Paths such as
//! `include_bytes!("../../target/static/telemetry/…")` resolve relative to a **source file** under
//! `core/src/…` (often to `core/target/…`), which is **not** the same directory as the workspace
//! `target/` used by `CARGO_TARGET_DIR`. That mismatch once produced binaries that compiled but
//! served empty or wrong `viz-lite.js`, breaking `importScripts` in the telemetry web worker.
//!
//! ### Flat filenames written to `OUT_DIR` (must stay in sync with `metrics_server.rs`)
//!
//! - `viz-lite.js.gz`
//! - `index.html.gz`
//! - `webworker.js.gz`
//! - `dot-viewer.js.gz`
//! - `dot-viewer.css.gz`
//! - `spinner.gif`
//!
//! ### Features
//! - `telemetry_server_builtin`: Embeds the telemetry server in the binary (offline-capable).
//! - `telemetry_server_cdn`: Uses a CDN for viz.js (smaller binary; needs internet in the browser).
//! - `telemetry_history`, `proactor_*`, etc.: See `Cargo.toml`.

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

/// Minimum size (bytes) for an acceptable `viz-lite.js.gz` under `OUT_DIR`. Smaller or missing
/// forces a re-download and re-compress so we never embed an empty or truncated artifact.
const VIZ_LITE_GZ_MIN_BYTES: u64 = 4096;

/// Output basename under `OUT_DIR` for the gzipped viz bundle (`metrics_server.rs` must match).
const OUT_VIZ_LITE_JS_GZ: &str = "viz-lite.js.gz";
/// Temporary download path under `OUT_DIR` (deleted after gzip).
const OUT_VIZ_LITE_DOWNLOAD_JS: &str = "viz-lite.download.js";

const OUT_INDEX_HTML_GZ: &str = "index.html.gz";
const OUT_WEBWORKER_JS_GZ: &str = "webworker.js.gz";
const OUT_DOT_VIEWER_JS_GZ: &str = "dot-viewer.js.gz";
const OUT_DOT_VIEWER_CSS_GZ: &str = "dot-viewer.css.gz";
const OUT_SPINNER_GIF: &str = "spinner.gif";

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

    // `OUT_DIR` is where embeddable build outputs **must** go so `include_bytes!(concat!(env!("OUT_DIR"), "/…"))`
    // in `metrics_server.rs` reads the same bytes this script just wrote. See module docs above.
    let out_dir = PathBuf::from(
        env::var("OUT_DIR").expect("OUT_DIR must be set by Cargo when running build.rs"),
    );

    if TELEMETRY_SERVICE && USE_INTERNAL_VIZ {
        // rustc embeds the result via `env!("OUT_DIR")`; do not move this output to `CARGO_TARGET_DIR` only.
        ensure_viz_lite_gz_in_out_dir(&out_dir, &format!("https://unpkg.com/viz.js@{}/viz-lite.js", VIZ_VERSION));
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

        let index_path = folder_base.join("index.html");

        // Check if we need to write the file
        let should_write = if Path::new(&index_path).exists() {
            match fs::read_to_string(&index_path) {
                Ok(existing_content) => existing_content != index_content,
                Err(_) => true, // If reading fails, assume it's different
            }
        } else {
            true // File doesn't exist, so need to write
        };

        let index_gz_out = out_dir.join(OUT_INDEX_HTML_GZ);
        let index_needs_gzip = should_write || !index_gz_out.exists();

        // Only write and gzip if necessary
        if should_write {
            fs::write(&index_path, &index_content).expect("Failed to write index.html");
        }
        gzip_source_to_out_dir(
            &out_dir,
            Path::new("static/telemetry/index.html"),
            OUT_INDEX_HTML_GZ,
            index_needs_gzip,
        );

        // Generate and write webworker.js from the template
        let webworker_content = WebWorkerTemplate { script_source: source }
            .render()
            .expect("Failed to render webworker.js template");
        let webworker_path = folder_base.join("webworker.js");

        // Check if we need to write the file
        let should_write = if Path::new(&webworker_path).exists() {
            match fs::read_to_string(&webworker_path) {
                Ok(existing_content) => existing_content != webworker_content,
                Err(_) => true, // If reading fails, assume it's different
            }
        } else {
            true // File doesn't exist, so need to write
        };

        let webworker_gz_out = out_dir.join(OUT_WEBWORKER_JS_GZ);
        let webworker_needs_gzip = should_write || !webworker_gz_out.exists();

        // Only write and gzip if necessary
        if should_write {
            fs::write(&webworker_path, &webworker_content).expect("Failed to write webworker.js");
        }
        gzip_source_to_out_dir(
            &out_dir,
            Path::new("static/telemetry/webworker.js"),
            OUT_WEBWORKER_JS_GZ,
            webworker_needs_gzip,
        );

        // Encode static files; only (re)compress when the `OUT_DIR` artifact is missing.
        gzip_source_to_out_dir(
            &out_dir,
            Path::new("static/telemetry/dot-viewer.js"),
            OUT_DOT_VIEWER_JS_GZ,
            !out_dir.join(OUT_DOT_VIEWER_JS_GZ).exists(),
        );
        gzip_source_to_out_dir(
            &out_dir,
            Path::new("static/telemetry/dot-viewer.css"),
            OUT_DOT_VIEWER_CSS_GZ,
            !out_dir.join(OUT_DOT_VIEWER_CSS_GZ).exists(),
        );

        // Copy spinner.gif into `OUT_DIR` under a flat name — this path is what `include_bytes!` uses.
        copy_spinner_into_out_dir(Path::new("static/telemetry/images/spinner.gif"), &out_dir.join(OUT_SPINNER_GIF));
    }
}

/// Downloads viz-lite if missing or too small under `OUT_DIR`, then gzip-encodes into `OUT_DIR`.
/// All final bytes for `include_bytes!` live under `out_dir`; never rely on `CARGO_TARGET_DIR` alone.
fn ensure_viz_lite_gz_in_out_dir(out_dir: &Path, get_url: &str) {
    let dest_gz = out_dir.join(OUT_VIZ_LITE_JS_GZ);
    let needs_refresh = !dest_gz.exists()
        || fs::metadata(&dest_gz)
            .map(|m| m.len() < VIZ_LITE_GZ_MIN_BYTES)
            .unwrap_or(true);

    if !needs_refresh {
        println!(
            "cargo:trace={:?} already present and large enough, skipping viz-lite fetch",
            dest_gz
        );
        return;
    }

    let _ = fs::remove_file(&dest_gz);
    let tmp_js = out_dir.join(OUT_VIZ_LITE_DOWNLOAD_JS);
    let mut response = isahc::get(get_url).expect("Failed to download viz-lite.js");
    let mut file = File::create(&tmp_js).expect("Failed to create temporary viz-lite.js under OUT_DIR");
    io::copy(&mut response.body_mut(), &mut file).expect("Failed to write downloaded viz-lite.js");
    drop(file);

    gzip_source_to_out_dir(out_dir, &tmp_js, OUT_VIZ_LITE_JS_GZ, false);
    let _ = fs::remove_file(&tmp_js);
}

/// Gzip-compresses `source_file` into `out_dir / out_gz_name`.
///
/// When `force` is false and the destination `.gz` already exists, does nothing (incremental builds).
/// When `force` is true, always (re)compresses so template edits cannot leave a stale gzip next to `OUT_DIR`.
/// Embeds must land under `out_dir` so `env!("OUT_DIR")` in the crate matches this path.
fn gzip_source_to_out_dir(out_dir: &Path, source_file: &Path, out_gz_name: &str, force: bool) {
    let dest_gz = out_dir.join(out_gz_name);
    if !force && dest_gz.exists() {
        println!("cargo:trace={:?} already exists, skipping compression", dest_gz);
        return;
    }

    if let Some(parent) = dest_gz.parent() {
        fs::create_dir_all(parent).expect("Failed to create OUT_DIR parent");
    }

    let mut input = File::open(source_file).unwrap_or_else(|e| {
        panic!("Failed to open source {:?} for gzip into OUT_DIR: {}", source_file, e)
    });
    let output = File::create(&dest_gz).expect("Failed to create gz file under OUT_DIR");
    let mut encoder = GzEncoder::new(output, Compression::default());
    io::copy(&mut input, &mut encoder).expect("Failed to compress into OUT_DIR");
    encoder.finish().expect("Failed to finalize gzip under OUT_DIR");

    println!("cargo:trace=Compressed {:?} -> {:?}", source_file, dest_gz);
}

/// Copies the repo’s spinner GIF into `OUT_DIR` for `include_bytes!`.
fn copy_spinner_into_out_dir(source_file: &Path, dest_file: &Path) {
    if dest_file.exists() {
        println!("cargo:trace={:?} already exists, skipping spinner copy", dest_file);
        return;
    }

    if let Some(parent) = dest_file.parent() {
        fs::create_dir_all(parent).expect("Failed to create OUT_DIR parent for spinner");
    }

    let mut source = File::open(source_file).expect("Failed to open spinner.gif source");
    let mut target = File::create(dest_file).expect("Failed to create spinner.gif under OUT_DIR");
    io::copy(&mut source, &mut target).expect("Failed to copy spinner.gif into OUT_DIR");

    println!("cargo:warning=Copied {:?} to {:?}", source_file, dest_file);
}
