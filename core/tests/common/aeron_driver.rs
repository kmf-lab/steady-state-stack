//! Media driver lifecycle helpers for integration tests (restart + probe).
#![allow(dead_code)]

use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use steady_state::media_driver_probe_with_reason;

use super::support::pub_sub_harness::{mark_suite_preflight_wire_verified, preflight_ipc_roundtrip};

fn aeron_dir() -> String {
    std::env::var("AERON_DIR").unwrap_or_else(|_| "/dev/shm/aeron-default".to_string())
}

fn systemd_unit() -> String {
    std::env::var("SS_AERON_SYSTEMD_UNIT").unwrap_or_else(|_| "aeronmd".to_string())
}

/// Matches `SS_AERON_POST_RESTART_SETTLE_SEC` in `scripts/run-aeron-integration.sh` (default 15).
fn post_restart_settle_sec() -> u64 {
    std::env::var("SS_AERON_POST_RESTART_SETTLE_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15)
}

fn wait_post_restart_settle() {
    let secs = post_restart_settle_sec();
    eprintln!("REFRESH: post-restart settle {secs}s (SS_AERON_POST_RESTART_SETTLE_SEC)");
    thread::sleep(Duration::from_secs(secs));
}

/// Same subprocess wire proof as `post_restart_wire_settle` in `run-aeron-integration.sh`.
fn run_post_restart_preflight_smoke() -> Result<(), String> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|e| format!("CARGO_MANIFEST_DIR unavailable: {e}"))?;
    let workspace_root = Path::new(&manifest_dir)
        .parent()
        .ok_or_else(|| format!("expected workspace root parent of {manifest_dir}"))?;

    eprintln!("REFRESH: post-restart preflight smoke (subprocess, stream 80000)");
    let status = Command::new("cargo")
        .current_dir(workspace_root)
        .env("SS_AERON_GATE_C", "1")
        .args([
            "test",
            "-p",
            "steady_state",
            "--test",
            "aeron_preflight_smoke",
            "aeron_preflight_wire_settle",
            "--",
            "--nocapture",
        ])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("failed to spawn aeron_preflight_smoke: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "aeron_preflight_smoke exited with {}",
            status.code().unwrap_or(-1)
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestartVia {
    Docker,
    Systemd,
    Binary,
}

fn docker_container_exists(unit: &str) -> bool {
    Command::new("docker")
        .args(["ps", "-a", "--format", "{{.Names}}"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|n| n == unit)
        })
        .unwrap_or(false)
}

fn systemd_unit_installed() -> bool {
    let unit = format!("{}.service", systemd_unit());
    Command::new("systemctl")
        .args(["list-unit-files", &unit, "--no-pager", "--no-legend"])
        .output()
        .map(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.lines().any(|line| line.starts_with(&unit))
        })
        .unwrap_or(false)
}

fn systemd_driver_active() -> bool {
    let unit = systemd_unit();
    Command::new("systemctl")
        .args(["is-active", "--quiet", &unit])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn spawn_path() -> Option<String> {
    let raw = resolve_aeronmd();
    let path = raw.trim_end_matches(" (deleted)").to_string();
    if path == "aeronmd" {
        return None;
    }
    let p = Path::new(&path);
    if p.is_file() && std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false) {
                return Some(path);
            }
            return None;
        }
        #[cfg(not(unix))]
        {
            return Some(path);
        }
    }
    None
}

fn detect_restart_via() -> RestartVia {
    match std::env::var("SS_AERON_RESTART_VIA")
        .unwrap_or_else(|_| "auto".to_string())
        .to_lowercase()
        .as_str()
    {
        "docker" => RestartVia::Docker,
        "systemctl" | "systemd" => RestartVia::Systemd,
        "binary" => RestartVia::Binary,
        "auto" | _ => {
            let unit = systemd_unit();
            if docker_container_exists(&unit) {
                RestartVia::Docker
            } else if systemd_unit_installed() || systemd_driver_active() {
                RestartVia::Systemd
            } else if spawn_path().is_some() {
                RestartVia::Binary
            } else {
                RestartVia::Systemd
            }
        }
    }
}

fn run_systemctl(args: &[&str]) -> Result<std::process::Output, String> {
    let unit = systemd_unit();
    let mut full_args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    full_args.push(unit);

    let direct = Command::new("systemctl")
        .args(&full_args)
        .output()
        .map_err(|e| format!("systemctl failed: {e}"))?;
    if direct.status.success() {
        return Ok(direct);
    }

    let mut sudo_args = vec!["-n".to_string(), "systemctl".to_string()];
    sudo_args.extend(full_args);
    Command::new("sudo")
        .args(&sudo_args)
        .output()
        .map_err(|e| format!("sudo systemctl failed: {e}"))
}

fn restart_via_docker() -> Result<(), String> {
    let unit = systemd_unit();
    if !docker_container_exists(&unit) {
        return Err(format!("docker container {unit} not found"));
    }
    eprintln!("REFRESH: docker restart {unit} (AERON_DIR={})", aeron_dir());
    let out = Command::new("docker")
        .args(["restart", &unit])
        .output()
        .map_err(|e| format!("docker restart failed: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    wait_post_restart_settle();
    settle_after_restart()
}

fn restart_via_systemd() -> Result<(), String> {
    let unit = systemd_unit();
    eprintln!("REFRESH: systemctl restart {unit} (AERON_DIR={})", aeron_dir());
    let systemctl_ok = run_systemctl(&["restart"])
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !systemctl_ok {
        return Err(format!("systemctl restart {unit} failed (try docker restart {unit})"));
    }
    wait_post_restart_settle();
    settle_after_restart()
}

fn restart_via_binary() -> Result<(), String> {
    let dir = aeron_dir();
    let spawn_path = spawn_path().ok_or_else(|| {
        format!(
            "no executable aeronmd (resolved {}); install systemd service or set AERONMD",
            resolve_aeronmd()
        )
    })?;

    eprintln!("REFRESH: restarting aeronmd ({spawn_path}, AERON_DIR={dir})...");
    let _ = Command::new("pkill")
        .args(["-f", "/aeronmd"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    thread::sleep(Duration::from_secs(2));

    Command::new(&spawn_path)
        .arg(format!("-Daeron.dir={dir}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn {spawn_path}: {e}"))?;

    thread::sleep(Duration::from_secs(2));
    wait_post_restart_settle();
    settle_after_restart()
}

fn settle_after_restart() -> Result<(), String> {
    media_driver_probe_with_reason(Duration::from_secs(20)).map_err(|e| format!("{e}"))?;
    thread::sleep(Duration::from_secs(3));
    if run_post_restart_preflight_smoke().is_ok() {
        mark_suite_preflight_wire_verified();
        eprintln!("REFRESH: aeronmd ready (CNC + subprocess preflight smoke)");
        return Ok(());
    }
    eprintln!("REFRESH: subprocess preflight smoke failed; falling back to in-process wire probe");
    preflight_ipc_roundtrip("driver_refresh").map_err(|e| format!("{e}"))?;
    mark_suite_preflight_wire_verified();
    eprintln!("REFRESH: aeronmd ready (CNC + in-process IPC wire probe with retries)");
    Ok(())
}

/// Resolve a spawnable `aeronmd` path (env, on-disk, or `/proc/pid/exe`).
pub fn resolve_aeronmd() -> String {
    if let Ok(path) = std::env::var("AERONMD") {
        let path = path.trim().to_string();
        if !path.is_empty() && Path::new(&path).exists() {
            return path;
        }
    }
    for candidate in [
        "/build/binaries/aeronmd",
        "/usr/local/bin/aeronmd",
        "/usr/bin/aeronmd",
    ] {
        if Path::new(candidate).is_file() {
            return candidate.to_string();
        }
    }
    if let Ok(output) = Command::new("pgrep").args(["-a", "-f", "/aeronmd"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some(pid) = line.split_whitespace().next() {
                let exe = format!("/proc/{pid}/exe");
                if let Ok(link) = std::fs::read_link(&exe) {
                    let link = link.to_string_lossy().trim_end_matches(" (deleted)").to_string();
                    if link.contains("aeronmd") && Path::new(&link).exists() {
                        return link;
                    }
                }
            }
            if let Some(path) = line.split_whitespace().find(|s| s.ends_with("/aeronmd")) {
                if Path::new(path).exists() {
                    return path.to_string();
                }
            }
        }
    }
    if let Ok(output) = Command::new("sh").args(["-c", "command -v aeronmd"]).output() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return path;
        }
    }
    "aeronmd".to_string()
}

fn refresh_minimal() -> bool {
    std::env::var("SS_AERON_REFRESH_MINIMAL")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Restart `aeronmd` and block until CNC + IPC wire probe succeed.
// ss[depends distributed.media-driver-testing]
// ss[related distributed.media-driver-testing]
pub fn refresh_media_driver() -> Result<(), String> {
    if refresh_minimal() {
        eprintln!("REFRESH: minimal mode (cooldown only, no driver restart)");
        thread::sleep(Duration::from_secs(3));
        return media_driver_probe_with_reason(Duration::from_secs(10)).map_err(|e| format!("{e}"));
    }

    match detect_restart_via() {
        RestartVia::Docker => restart_via_docker(),
        RestartVia::Systemd => restart_via_systemd(),
        RestartVia::Binary => restart_via_binary().or_else(|e| {
            if docker_container_exists(&systemd_unit()) {
                eprintln!("REFRESH: binary restart failed ({e}); trying docker");
                restart_via_docker()
            } else if systemd_unit_installed() || systemd_driver_active() {
                eprintln!("REFRESH: binary restart failed ({e}); trying systemctl");
                restart_via_systemd()
            } else {
                Err(e)
            }
        }),
    }
}
