use std::process::{Command, Child, Stdio};
use std::path::{Path, PathBuf};
use std::fs;
use std::thread::{sleep, spawn};
use std::time::{Duration, Instant};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::sync::Mutex;
use std::io::{self, Write};

const USE_CUSTOM_CARGO_BUILD: bool = true;   //#!#//
const CLEAN_BEFORE_BUILD: bool = false;   //#!#//
const LAUNCH_DELAY_SECS: u64 = 7;   //#!#//

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum BuildMode {
    #[allow(dead_code)]
    Debug,
    #[allow(dead_code)]
    Release,
    #[allow(dead_code)]
    External, // Use pre-built binary, don't build
}

#[derive(Copy, Clone, Debug)]
struct PodSpec {
    name: &'static str,
    build_mode: BuildMode,
}

const POD_CONFIG: &[PodSpec] = &[
    PodSpec { name: "publish", build_mode: BuildMode::Release },
    PodSpec { name: "subscribe", build_mode: BuildMode::Release },
];

fn build_time_path(pod: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("steady_state_build_{}.txt", pod));
    p
}

fn read_last_build_time(pod: &str) -> Option<String> {
    let path = build_time_path(pod);
    fs::read_to_string(path).ok()
}

fn write_build_time(pod: &str, elapsed: Duration) {
    let path = build_time_path(pod);
    let _ = fs::write(path, format!("{:.2?}", elapsed));
}

// New function to parse the build time string (e.g., "1.23s") into a Duration
fn parse_build_time(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.ends_with('s') {
        let num_str = &s[..s.len() - 1];
        num_str.parse::<f64>().ok().map(Duration::from_secs_f64)
    } else {
        None
    }
}

fn main() {
    let exe_suffix = if cfg!(windows) { ".exe" } else { "" };

    println!(
        "Steady State Pod Runner\n\
         ======================\n\
         USE_CUSTOM_CARGO_BUILD: {USE_CUSTOM_CARGO_BUILD}\n\
         CLEAN_BEFORE_BUILD: {CLEAN_BEFORE_BUILD}\n\
         LAUNCH_DELAY_SECS: {LAUNCH_DELAY_SECS}\n\
         exe_suffix: {exe_suffix}\n"
    );

    // Ctrl-C handling: kill all children and exit
    let children_arc: Arc<Mutex<Vec<Child>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let children_arc = Arc::clone(&children_arc);
        ctrlc::set_handler(move || {
            println!("\nCtrl-C received, terminating all pods...");
            let mut children = children_arc.lock().unwrap();
            for child in children.iter_mut() {
                let _ = child.kill();
            }
            std::process::exit(130);
        }).expect("Error setting Ctrl-C handler");
    }

    if USE_CUSTOM_CARGO_BUILD && CLEAN_BEFORE_BUILD {
        println!("Running `cargo clean`...");
        let status = Command::new("cargo")
            .arg("clean")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("Failed to invoke cargo clean");
        if !status.success() {
            eprintln!("ERROR: Cargo clean failed.");
            return;
        }
        println!("Cargo clean complete.\n");
    }

    // === PHASE 1: BUILD ALL PODS FIRST ===
    if USE_CUSTOM_CARGO_BUILD {
        for pod in POD_CONFIG.iter() {
            if pod.build_mode == BuildMode::External {
                continue;
            }
            let profile = match pod.build_mode {
                BuildMode::Debug => "debug",
                BuildMode::Release => "release",
                BuildMode::External => unreachable!(),
            };
            let mut args = vec!["build"];
            if profile == "release" {
                args.push("--release");
            }
            args.push("--bin");
            args.push(pod.name);

            if let Some(last) = read_last_build_time(pod.name) {
                println!("Last build time for '{}': {last}", pod.name);
            }

            println!("Building pod '{}' (cargo {:?})...", pod.name, args);

            // Modified progress indicator setup
            let spinning = Arc::new(AtomicBool::new(true));
            let spinner_handle = {
                let spinning = Arc::clone(&spinning);
                let start = Instant::now();
                let previous_build_time = read_last_build_time(pod.name).and_then(|s| parse_build_time(&s));
                spawn(move || {
                    while spinning.load(Ordering::Relaxed) {
                        let elapsed = start.elapsed();
                        if let Some(prev) = previous_build_time {
                            if prev > Duration::ZERO {
                                let percentage = ((elapsed.as_secs_f64() / prev.as_secs_f64()) * 100.0).round() as u32;
                                print!("\rCompiling... {}%", percentage);
                            } else {
                                print!("\rCompiling...");
                            }
                        } else {
                            print!("\rCompiling...");
                        }
                        io::stdout().flush().ok();
                        sleep(Duration::from_millis(100));
                    }
                    let elapsed = start.elapsed();
                    print!("\rBuild complete in {:.2?}                    \n", elapsed);
                    io::stdout().flush().ok();
                })
            };

            let start = Instant::now();
            let output = Command::new("cargo")
                .args(&args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .expect("Failed to invoke cargo build");
            let elapsed = start.elapsed();

            spinning.store(false, Ordering::Relaxed);
            spinner_handle.join().ok();

            write_build_time(pod.name, elapsed);

            if !output.status.success() {
                eprintln!("\nERROR: Cargo build failed for pod '{}'.", pod.name);
                eprintln!("--- Cargo stdout ---");
                eprintln!("{}", String::from_utf8_lossy(&output.stdout));
                eprintln!("--- Cargo stderr ---");
                eprintln!("{}", String::from_utf8_lossy(&output.stderr));
                eprintln!("Aborting: No pods will be launched.");
                return;
            }
        }
        println!("\nAll pods built successfully.\n");
    }

    // === PHASE 2: LAUNCH ALL PODS IN ORDER ===
    for (i, pod) in POD_CONFIG.iter().enumerate() {
        let profile = match pod.build_mode {
            BuildMode::Debug => "debug",
            BuildMode::Release => "release",
            BuildMode::External => {
                if Path::new(&format!("./target/release/{}{}", pod.name, exe_suffix)).exists() {
                    "release"
                } else {
                    "debug"
                }
            }
        };

        let bin_path = format!("./target/{profile}/{}{}", pod.name, exe_suffix);

        println!("\nAttempting to launch: {bin_path}");

        let path = Path::new(&bin_path);

        let mut current = PathBuf::new();
        let mut found = true;
        for comp in path.components() {
            current.push(comp);
            if !current.exists() {
                found = false;
                eprintln!("ERROR: Path component not found: {}", current.display());
                if let Some(parent) = current.parent() {
                    println!("Listing contents of parent: {}", parent.display());
                    match fs::read_dir(parent) {
                        Ok(entries) => {
                            for entry in entries.flatten() {
                                println!("  - {}", entry.file_name().to_string_lossy());
                            }
                        }
                        Err(e) => {
                            println!("  (Could not read directory: {e})");
                        }
                    }
                } else {
                    println!("No parent directory to list.");
                }
                break;
            }
        }

        if !found {
            continue;
        }

        match Command::new(&path).spawn() {
            Ok(child) => {
                println!("Launched: {bin_path}");
                children_arc.lock().unwrap().push(child);
            }
            Err(e) => {
                eprintln!("ERROR: Failed to start {bin_path}: {e}");
            }
        }

        if i + 1 < POD_CONFIG.len() {
            println!("Waiting {LAUNCH_DELAY_SECS} second(s) before launching next pod...");
            sleep(Duration::from_secs(LAUNCH_DELAY_SECS));
        }
    }

    // Wait for all children to finish
    let mut children = children_arc.lock().unwrap();
    for (i, mut child) in children.drain(..).enumerate() {
        match child.wait() {
            Ok(status) => println!("Process {i} exited with: {status}"),
            Err(e) => eprintln!("ERROR: Failed to wait for process {i}: {e}"),
        }
    }
}