<#
.SYNOPSIS
    Installs Aeron C++ Media Driver on Windows 11, including all prerequisites, for high-performance messaging in the steady_state crate.

.DESCRIPTION
    This script automates the installation of Chocolatey, required tools (Visual Studio with C++ workload, CMake, Git, NSSM, zlib),
    clones the Aeron repository, builds the C++ Media Driver, sets it up as a Windows service, and applies performance tweaks.

.NOTES
    - Must be run as Administrator.
    - Designed for Windows 11.
    - Installs Aeron version 1.46.7 with zlib for compression support.
    - Ensures C:\Temp exists for logs and shared memory.
#>

# --- 0. Ensure Running as Administrator ---
#if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
#    Write-Host "Script requires Administrator privileges. Relaunching as Admin..."
#    Start-Process powershell -Verb RunAs -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`""
#    Exit
#}

# --- 1. Install Chocolatey if not already installed ---
if (-not (Get-Command choco -ErrorAction SilentlyContinue)) {
    Write-Host "Chocolatey not found. Installing Chocolatey..."
    Set-ExecutionPolicy Bypass -Scope Process -Force
    [System.Net.ServicePointManager]::SecurityProtocol = [System.Net.ServicePointManager]::SecurityProtocol -bor 3072
    iex ((New-Object System.Net.WebClient).DownloadString('https://chocolatey.org/install.ps1'))
    # Refresh environment variables
    $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")
    Write-Host "Chocolatey installed successfully."
}

# --- 2. Install Required Tools via Chocolatey ---
Write-Host "Installing Visual Studio 2022 with C++ workload..."
choco install visualstudio2022community -y --package-parameters "--add Microsoft.VisualStudio.Workload.NativeDesktop --includeRecommended"

Write-Host "Installing CMake..."
choco install cmake -y

Write-Host "Installing Git..."
choco install git -y

Write-Host "Installing NSSM..."
choco install nssm -y

Write-Host "Installing zlib (compression library)..."
choco install zlib -y

# --- 3. Verify Installations ---
Write-Host "Verifying installations..."
$tools = @(
    @{Name="CMake"; Command="cmake"},
    @{Name="Git"; Command="git"},
    @{Name="NSSM"; Command="nssm"}
# zlib is a library, so we can't directly check for a command; assume choco install succeeded
)
foreach ($tool in $tools) {
    if (-not (Get-Command $tool.Command -ErrorAction SilentlyContinue)) {
        Write-Error "$($tool.Name) installation failed. Please check logs and try again."
        Exit 1
    }
    Write-Host "$($tool.Name) is installed."
}

# --- 4. Ensure C:\Temp exists for logs and shared memory ---
$tempDir = "C:\Temp"
if (-not (Test-Path $tempDir)) {
    Write-Host "[INFO] Creating directory: $tempDir"
    New-Item -ItemType Directory -Path $tempDir | Out-Null
}

# --- 5. Clone or Update Aeron Repository ---
Write-Host "`n=== [1] Clone or Pull Aeron Repository ==="
$AERON_VERSION = '1.46.7'
$AERON_SRC_PATH = Join-Path (Join-Path $env:USERPROFILE 'build') 'aeron'

if (-not (Test-Path "$AERON_SRC_PATH\.git")) {
    Write-Host "Cloning Aeron ($AERON_VERSION) into $AERON_SRC_PATH..."
    git clone --depth 1 --branch $AERON_VERSION https://github.com/real-logic/aeron.git $AERON_SRC_PATH
    if ($LASTEXITCODE -ne 0) { Write-Error "Git clone failed."; Exit 1 }
} else {
    Write-Host "Aeron repo already present. Updating to $AERON_VERSION..."
    Push-Location $AERON_SRC_PATH
    git fetch --all --tags
    git checkout $AERON_VERSION
    git pull
    Pop-Location
}

# --- 6. Build Aeron (C++ Only, Java Disabled) ---
Write-Host "`n=== [2] Build Aeron (C++ only, Java disabled) ==="
$BUILD_FOLDER = Join-Path (Join-Path $env:USERPROFILE 'build') 'aeron-build'
if (-not (Test-Path $BUILD_FOLDER)) {
    Write-Host "[INFO] Creating build output folder: $BUILD_FOLDER"
    New-Item -ItemType Directory -Path $BUILD_FOLDER | Out-Null
}

Push-Location $BUILD_FOLDER

# Configure CMake: Use Visual Studio 2022, 64-bit, disable Java, build samples, source from $AERON_SRC_PATH
cmake -G "Visual Studio 17 2022" -A x64 -DCMAKE_BUILD_TYPE=Release -DAERON_ENABLE_JAVA=OFF -DAERON_BUILD_SAMPLES=ON -DAERON_BUILD_TOOLS=OFF $AERON_SRC_PATH

if ($LASTEXITCODE -ne 0) {
    Write-Error "[ERROR] CMake configuration failed. Ensure Visual Studio and CMake are installed."
    Exit 1
}

# Build the targets: aeronmd and sample apps
cmake --build . --config Release --target aeronmd basic_publisher basic_subscriber streaming_publisher
if ($LASTEXITCODE -ne 0) {
    Write-Error "[ERROR] CMake build failed. Check Visual Studio C++ workload."
    Exit 1
}

Pop-Location

# --- 7. Install or Update Service via NSSM ---
Write-Host "`n=== [3] Install or Update Windows Service ==="
$SERVICE_NAME = 'AeronMediaDriver'
$SERVICE_DESC = 'Aeron C Media Driver Service'
$NSSM_EXE = 'nssm'
$exeFullPath = Join-Path (Join-Path (Join-Path $BUILD_FOLDER 'binaries') 'Release') 'aeronmd.exe'

if (-not (Test-Path $exeFullPath)) {
    Write-Error "[ERROR] aeronmd.exe not found at path: $exeFullPath"
    Exit 1
}

# Ensure AERON_DIR exists for shared memory
$AERON_DIR = "C:\Temp\aeron"
if (-not (Test-Path $AERON_DIR)) {
    Write-Host "[INFO] Creating directory for Aeron shared memory: $AERON_DIR"
    New-Item -ItemType Directory -Path $AERON_DIR | Out-Null
}

# Define environment variables for Aeron configuration
$envVars = @{
    "AERON_DIR" = $AERON_DIR
    "AERON_THREADING_MODE" = "DEDICATED"  # Dedicated threads for better performance
    "AERON_SOCKET_SO_RCVBUF" = "4194304"  # 4MB receive buffer
    "AERON_SOCKET_SO_SNDBUF" = "4194304"  # 4MB send buffer
    "AERON_EVENT_LOG_FILENAME" = "$tempDir\\aeron_event.log"  # Event log location
}

# Convert environment variables to NSSM format
$envString = ($envVars.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join " "

$serviceObj = Get-Service -Name $SERVICE_NAME -ErrorAction SilentlyContinue

if (-not $serviceObj) {
    Write-Host "[INFO] Installing service '$SERVICE_NAME' via NSSM..."
    & $NSSM_EXE install $SERVICE_NAME $exeFullPath
    if ($LASTEXITCODE -ne 0) {
        Write-Error "[ERROR] NSSM install failed. Ensure nssm.exe is correctly configured."
        Exit 1
    }

    # Configure service settings
    & $NSSM_EXE set $SERVICE_NAME Description $SERVICE_DESC
    & $NSSM_EXE set $SERVICE_NAME Start SERVICE_AUTO_START  # Auto-start on boot
    & $NSSM_EXE set $SERVICE_NAME AppThrottle 0
    & $NSSM_EXE set $SERVICE_NAME AppRestartDelay 5000  # 5-second delay before restart
    & $NSSM_EXE set $SERVICE_NAME AppStdout "$tempDir\\aeron_stdout.log"
    & $NSSM_EXE set $SERVICE_NAME AppStderr "$tempDir\\aeron_stderr.log"
    & $NSSM_EXE set $SERVICE_NAME AppRotateFiles 1  # Rotate logs
    & $NSSM_EXE set $SERVICE_NAME AppExit Default Restart  # Restart on exit
    & $NSSM_EXE set $SERVICE_NAME AppEnvironmentExtra $envString
    & $NSSM_EXE set $SERVICE_NAME AppDirectory (Split-Path $exeFullPath -Parent)

    Write-Host "✅ Service installed successfully."
} else {
    Write-Host "[INFO] Service already exists. Updating configuration..."
    & $NSSM_EXE set $SERVICE_NAME Application $exeFullPath
    & $NSSM_EXE set $SERVICE_NAME AppEnvironmentExtra $envString
    & $NSSM_EXE set $SERVICE_NAME AppDirectory (Split-Path $exeFullPath -Parent)
}

# Start the service if not running
$serviceObj = Get-Service -Name $SERVICE_NAME
if ($serviceObj.Status -ne 'Running') {
    Write-Host "[INFO] Starting service..."
    & $NSSM_EXE start $SERVICE_NAME
    if ($LASTEXITCODE -ne 0) {
        Write-Error "[ERROR] Failed to start service '$SERVICE_NAME'."
        Exit 1
    }
    Write-Host "✅ Service started."
} else {
    Write-Host "[INFO] Service is already running."
}

# --- 8. Apply Windows Registry Tweaks for UDP Performance ---
Write-Host "`n=== [4] Apply Windows Registry Tweaks (UDP Performance) ==="
$SOCKET_BUFFER_HEX = 0x400000  # 4MB in hex for registry tweaks
reg add "HKLM\SYSTEM\CurrentControlSet\Services\Afd\Parameters" /v DefaultSendWindow /t REG_DWORD /d $SOCKET_BUFFER_HEX /f | Out-Null
reg add "HKLM\SYSTEM\CurrentControlSet\Services\Afd\Parameters" /v DefaultReceiveWindow /t REG_DWORD /d $SOCKET_BUFFER_HEX /f | Out-Null
reg add "HKLM\SYSTEM\CurrentControlSet\Services\Afd\Parameters" /v FastSendDatagramThreshold /t REG_DWORD /d 1500 /f | Out-Null
reg add "HKLM\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters" /v MaximumReassemblyHeaders /t REG_DWORD /d 0xFFFF /f | Out-Null
reg add "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile" /v NetworkThrottlingIndex /t REG_DWORD /d 0xFFFFFFFF /f | Out-Null

Write-Host "`n[INFO] Registry tuning applied. A reboot may be required."

# --- 9. Final Instructions ---
Write-Host "`n=== ✅ Setup Complete ==="
Write-Host "Aeron source code:  '$AERON_SRC_PATH'"
Write-Host "Build directory:    '$BUILD_FOLDER'"
Write-Host "Service name:       '$SERVICE_NAME'"
Write-Host "To check status:    sc.exe query $SERVICE_NAME"
Write-Host "To stop service:    sc.exe stop $SERVICE_NAME"
Write-Host "To remove service:  nssm.exe remove $SERVICE_NAME"
Write-Host "To check logs:      Get-Content $tempDir\\aeron_stdout.log -Tail 20 -Wait"
Write-Host "                   Get-Content $tempDir\\aeron_stderr.log -Tail 20 -Wait"
Write-Host "                   Get-Content $tempDir\\aeron_event.log -Tail 20 -Wait"