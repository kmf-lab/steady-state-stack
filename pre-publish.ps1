# Requires PowerShell 5.x
# Equivalent to the bash-based prepublish script
# Usage: powershell -ExecutionPolicy Bypass -File .\prepublish.ps1

Write-Host "================================================================================"
Write-Host " Rust Prepublish Script (PowerShell 5.x Compatible)"
Write-Host "================================================================================"

# Ensure script stops on first error
$ErrorActionPreference = "Stop"

function Check-Exit($exitCode, $message) {
    if ($exitCode -ne 0) {
        Write-Host "ERROR: $message (exit code: $exitCode)" -ForegroundColor Red
        exit $exitCode
    }
}

Write-Host "`n[Step] Updating cargo dependencies and ensuring stable toolchain..."
Start-Process "cargo" "update" -Wait -NoNewWindow
Start-Process "rustup" "default stable" -Wait -NoNewWindow

# Check if sccache is available
$hasSccache = $false
try {
    & sccache --version | Out-Null
    $hasSccache = $true
} catch {
    $hasSccache = $false
}

if ($hasSccache) {
    Write-Host "USING sccache ---------------------------------------"
    $Env:RUSTC_WRAPPER = "sccache"
}

# Check for unwanted crates
$unwantedCrates = @("tokio", "actix", "rocket", "warp")
Write-Host "`n[Step] Checking for unwanted crates..."
$cargoTree = & cargo tree

foreach ($crate in $unwantedCrates) {
    if ($cargoTree -match $crate) {
        Write-Host "Error: '$crate' crate found in the Cargo project." -ForegroundColor Red
        $cargoTree | Select-String $crate -Context 0,15
        exit 1
    } else {
        Write-Host "Success: No '$crate' crate found in the Cargo project."
    }
}

# Ensure cargo-nextest is installed
Write-Host "`n[Step] Checking for cargo-nextest..."
$cargoNextestExists = $false
try {
    & cargo-nextest --version | Out-Null
    $cargoNextestExists = $true
} catch {
    $cargoNextestExists = $false
}

if (-not $cargoNextestExists) {
    Write-Host "cargo-nextest not found, installing..."
    Start-Process "cargo" "install cargo-nextest" -Wait -NoNewWindow
}

# Run tests with cargo-nextest
Write-Host "`n[Step] Running tests with cargo-nextest..."
$Env:RUST_BACKTRACE = "full"
$Env:RUST_LOG = "debug"

& cargo nextest run --workspace --examples --tests | Tee-Object -FilePath "cargo_test.txt"
Check-Exit $LASTEXITCODE "Tests failed"

Write-Host "`nAll tests passed successfully."
Write-Host "==========================================================================="

# Build debug
Write-Host "`n[Step] Building workspace (debug)..."
$Env:RUST_BACKTRACE = "1"
& cargo build --offline --workspace | Tee-Object -FilePath "cargo_build.txt"
Check-Exit $LASTEXITCODE "Debug build failed"

# Build release
Write-Host "`n[Step] Building workspace (release, multi-core)..."
& cargo build --release --workspace --examples --tests -j 12 | Tee-Object -FilePath "cargo_build_release.txt"
Check-Exit $LASTEXITCODE "Release build failed"

# Build documentation
Write-Host "`n[Step] Building rustdoc..."
Push-Location "core"
$Env:RUSTDOCFLAGS = "--cfg=docsrs"
& cargo rustdoc | Tee-Object -FilePath "cargo_rustdoc.txt"
Check-Exit $LASTEXITCODE "Docs build failed"
Pop-Location

# Optional directory tree (requires tree.exe or Get-ChildItem fallback)
if (Get-Command tree -ErrorAction SilentlyContinue) {
    tree /F /A | Where-Object {$_ -notmatch "target"}
} else {
    Write-Host "`n[Step] Directory structure (excluding target/):"
    Get-ChildItem -Recurse | Where-Object { $_.FullName -notmatch "target" } | Select-String ""
}

# Cargo outdated and audit
Write-Host "`n[Step] Checking for outdated dependencies..."
& cargo install cargo-outdated | Out-Null
& cargo outdated | Tee-Object -FilePath "cargo_outdated.txt"

Write-Host "`n[Step] Running cargo audit..."
& cargo audit | Tee-Object -FilePath "cargo_audit.txt"

# Optional: cargo-steady-state current build
Write-Host "`n[Step] Installing cargo-steady-state..."
& cargo install --path cargo-steady-state

# Coverage section
Write-Host "`n---------------------------------------------------------------------------------"
Write-Host "------------------------    compute coverage please wait ------------------------"
Write-Host "---------------------------------------------------------------------------------"

& cargo llvm-cov --lcov --output-path cov_a.lcov --no-default-features -F exec_async_std,telemetry_server_builtin,core_affinity,core_display,prometheus_metrics
& cargo llvm-cov --lcov --output-path cov_b.lcov --no-default-features -F proactor_nuclei,telemetry_server_cdn

& lcov --add-tracefile cov_a.lcov --add-tracefile cov_b.lcov -o merged.lcov
& genhtml merged.lcov --output-directory coverage_html

# List code statistics
Write-Host "`n[Step] Running tokei..."
& tokei | Tee-Object -FilePath "cargo_tokei.txt"

Write-Host "`n---------------------------------------------------------------------------------"
Write-Host "Confirm that warnings you do not want published have been removed."
Write-Host "If this is confirmed by successful GitHub build, you may now run: cargo publish"
Write-Host "---------------------------------------------------------------------------------"