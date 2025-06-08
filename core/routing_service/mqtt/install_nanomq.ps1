<#
.SYNOPSIS
    Installs NanoMQ MQTT Broker on Windows 11 with detailed logging for debugging runtime issues.

.DESCRIPTION
    This script automates the installation of prerequisites (Chocolatey, Visual Studio Build Tools, CMake, Git, NSSM),
    clones and builds NanoMQ from source, sets it up as a Windows service using NSSM, and provides logging.

.NOTES
    - Must be run as Administrator.
    - Designed for Windows 11.
    - Logs are saved to C:\Temp\Setup-NanoMQService.log and service logs in C:\Temp.
#>

$global:setupLogFile = "C:\Temp\Setup-NanoMQService.log"
if (-not (Test-Path "C:\Temp")) { New-Item -ItemType Directory -Path "C:\Temp" | Out-Null }
if (Test-Path $setupLogFile) { Remove-Item $setupLogFile -Force }
Start-Transcript -Path $setupLogFile -Append

function Log-Message {
    param (
        [string]$message,
        [string]$level = "INFO"
    )
    $timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    $logEntry = "[$timestamp] [$level] $message"
    Write-Host $logEntry
}

function Fail-And-Exit {
    param([string]$msg)
    Log-Message $msg "ERROR"
    Write-Host "`nERROR: $msg"
    Write-Host "See the full log at $global:setupLogFile"
    Stop-Transcript | Out-Null
    Exit 1
}

try {
    Log-Message "Starting NanoMQ installation script."

    # --- 0. Ensure Running as Administrator ---
    Log-Message "Checking if script is running as Administrator..."
    if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        Log-Message "Script is not running as Administrator. Attempting to relaunch..." "WARNING"
        Start-Process powershell -Verb RunAs -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`""
        Stop-Transcript | Out-Null
        Exit
    } else {
        Log-Message "Script is running with Administrator privileges."
    }

    # --- 1. Install Chocolatey if not already installed ---
    Log-Message "Checking for Chocolatey installation..."
    if (-not (Get-Command choco -ErrorAction SilentlyContinue)) {
        Log-Message "Chocolatey not found. Installing Chocolatey..."
        Set-ExecutionPolicy Bypass -Scope Process -Force
        [System.Net.ServicePointManager]::SecurityProtocol = [System.Net.ServicePointManager]::SecurityProtocol -bor 3072
        iex ((New-Object System.Net.WebClient).DownloadString('https://chocolatey.org/install.ps1'))
        if ($LASTEXITCODE -ne 0) {
            Fail-And-Exit "Chocolatey installation failed."
        }
        $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")
        Log-Message "Chocolatey installed successfully."
    } else {
        Log-Message "Chocolatey is already installed."
    }

    # --- 2. Install Required Tools via Chocolatey ---
    Log-Message "Installing required tools via Chocolatey..."
    $toolsToInstall = @(
        @{Name="Visual Studio 2022 Build Tools"; Package="visualstudio2022buildtools"; Params="--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"},
        @{Name="CMake"; Package="cmake"},
        @{Name="Git"; Package="git"},
        @{Name="NSSM"; Package="nssm"}
    )

    foreach ($tool in $toolsToInstall) {
        Log-Message "Installing $($tool.Name)..."
        $installCmd = "choco install $($tool.Package) -y"
        if ($tool.Params) { $installCmd += " --package-parameters `"$($tool.Params)`"" }
        Invoke-Expression $installCmd
        if ($LASTEXITCODE -ne 0) {
            Fail-And-Exit "Failed to install $($tool.Name)."
        }
        Log-Message "$($tool.Name) installed successfully."
    }

    # --- 3. Verify Installations ---
    Log-Message "Verifying installations..."
    $toolsToVerify = @(
        @{Name="CMake"; Command="cmake"},
        @{Name="Git"; Command="git"},
        @{Name="NSSM"; Command="nssm"}
    )
    foreach ($tool in $toolsToVerify) {
        if (-not (Get-Command $tool.Command -ErrorAction SilentlyContinue)) {
            Fail-And-Exit "$($tool.Name) is not installed or not in PATH."
        } else {
            Log-Message "$($tool.Name) is installed."
        }
    }

    # --- 4. Clone and Build NanoMQ ---
    $NANOMQ_VERSION = "0.21.7"
    $NANOMQ_SRC_PATH = Join-Path $env:USERPROFILE "build\nanomq"
    $NANOMQ_BUILD_PATH = Join-Path $env:USERPROFILE "build\nanomq-build"
    $NANOMQ_EXE = Join-Path $NANOMQ_BUILD_PATH "Release\nanomq.exe"

    Log-Message "Cleaning previous NanoMQ source and build directories..."
    Remove-Item -Recurse -Force $NANOMQ_SRC_PATH -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $NANOMQ_BUILD_PATH -ErrorAction SilentlyContinue

    Log-Message "Cloning NanoMQ ($NANOMQ_VERSION) into $NANOMQ_SRC_PATH..."
    git clone --depth 1 --branch v$NANOMQ_VERSION https://github.com/nanomq/nanomq.git $NANOMQ_SRC_PATH
    if ($LASTEXITCODE -ne 0) {
        Fail-And-Exit "Git clone failed."
    }
    Log-Message "NanoMQ repository cloned successfully."

    Log-Message "Creating build directory: $NANOMQ_BUILD_PATH"
    New-Item -ItemType Directory -Path $NANOMQ_BUILD_PATH | Out-Null

    Push-Location $NANOMQ_BUILD_PATH

    Log-Message "Configuring NanoMQ with CMake..."
    cmake -G "Visual Studio 17 2022" -A x64 -DCMAKE_BUILD_TYPE=Release $NANOMQ_SRC_PATH | Tee-Object -FilePath "C:\Temp\nanomq_cmake.log" -Append
    if ($LASTEXITCODE -ne 0) {
        Pop-Location
        Fail-And-Exit "CMake configuration failed. See log: C:\Temp\nanomq_cmake.log"
    }
    Log-Message "CMake configuration successful."

    Log-Message "Building NanoMQ (Release)..."
    cmake --build . --config Release | Tee-Object -FilePath "C:\Temp\nanomq_build.log" -Append
    if ($LASTEXITCODE -ne 0) {
        Pop-Location
        Fail-And-Exit "NanoMQ build failed. See log: C:\Temp\nanomq_build.log"
    }
    Pop-Location

    if (-not (Test-Path $NANOMQ_EXE)) {
        Fail-And-Exit "Could not find nanomq.exe after build! See C:\Temp\nanomq_build.log"
    }
    Log-Message "Found nanomq.exe at $NANOMQ_EXE"

    # --- 5. Install or Update Service via NSSM ---
    $SERVICE_NAME = 'NanoMQ'
    $NSSM_EXE = 'nssm'

    # Remove existing service if present
    if (Get-Service -Name $SERVICE_NAME -ErrorAction SilentlyContinue) {
        Log-Message "Stopping and removing existing service: $SERVICE_NAME"
        & $NSSM_EXE stop $SERVICE_NAME
        Start-Sleep -Seconds 2
        & $NSSM_EXE remove $SERVICE_NAME confirm
        Start-Sleep -Seconds 2
    }

    # Install NanoMQ as a service
    Log-Message "Installing NanoMQ service via NSSM..."
    & $NSSM_EXE install $SERVICE_NAME $NANOMQ_EXE
    & $NSSM_EXE set $SERVICE_NAME AppDirectory $NANOMQ_BUILD_PATH
    & $NSSM_EXE set $SERVICE_NAME AppStdout "C:\Temp\NanoMQService.log"
    & $NSSM_EXE set $SERVICE_NAME AppStderr "C:\Temp\NanoMQService.err.log"
    & $NSSM_EXE set $SERVICE_NAME Start SERVICE_AUTO_START

    # Start the service
    Log-Message "Starting NanoMQ service..."
    & $NSSM_EXE start $SERVICE_NAME

    Log-Message "NanoMQ installation and service setup complete."
    Write-Host "`nNanoMQ is installed and running as a Windows service ($SERVICE_NAME)."
    Write-Host "Check logs in C:\Temp\NanoMQService.log"
    Write-Host "Full setup log: $global:setupLogFile"
    Stop-Transcript | Out-Null
}
catch {
    Write-Host "`nERROR: $($_.Exception.Message)"
    Write-Host "See the full log at $global:setupLogFile"
    Stop-Transcript | Out-Null
    Exit 1
}