<#
.SYNOPSIS
    Clones or updates the Aeron repository into a user-specific source directory,
    builds the C++ Media Driver (with Java disabled) in a separate build directory,
    installs aeronmd.exe as a Windows service using NSSM, and applies performance tweaks.

.DESCRIPTION
    This script automates the setup of the Aeron Media Driver as a Windows service.
    It handles repository cloning, building the driver, service installation, and
    UDP performance tuning.

.PARAMETER Force
    Forces a rebuild of the Aeron Media Driver even if aeronmd.exe already exists.

.NOTES
    - **Run as Administrator**: This script must be executed from an elevated PowerShell prompt.
    - **Purpose**: Sets up the Aeron Media Driver for high-performance messaging on Windows.

    - **How to Clear It for Use (Prepare the Environment)**:
        1. Install Visual Studio with the C++ workload (e.g., "Desktop development with C++").
        2. Install CMake and add it to your system PATH (e.g., via `choco install cmake`).
        3. Install Git and ensure it's in your PATH (e.g., via `choco install git`).
        4. Download NSSM and place nssm.exe in a PATH directory (e.g., C:\Windows\System32).
        5. Verify prerequisites: Run `cmake --version`, `git --version`, and `nssm` in a terminal.
        6. Ensure C:\Temp exists and is writable by the SYSTEM account (created automatically).

    - **How to Use It Well (Run and Troubleshoot)**:
        - Run the script: `.\Setup-AeronService.ps1` or `.\Setup-AeronService.ps1 -Force` to rebuild.
        - Check output: Look for "✅" markers indicating success.
        - Monitor logs: Stdout/Stderr/Event logs are in C:\Temp (e.g., `Get-Content C:\Temp\aeron_stdout.log -Tail 20 -Wait`).
        - Test manually: Use the commands printed after service setup to run aeronmd.exe directly.
        - Troubleshoot:
            - Service not starting? Check C:\Temp\aeron_stderr.log and prerequisites.
            - No logs? Verify C:\Temp is writable.
            - Use `sc.exe query AeronMediaDriver` to check service status.
        - **Locations**: Source code in C:\Users\<username>\build\aeron, build outputs in C:\Users\<username>\build\aeron-build

.EXAMPLE
    .\Setup-AeronService.ps1 -Force
    # Forces a rebuild and reinstalls the service.
#>

param(
    [switch]$Force
)

# Capture the script start time for log filtering
$scriptStartTime = Get-Date

# --- 0. Ensure Running as Administrator ---
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole] "Administrator")) {
    Write-Host "[ERROR] Please run this script in an Administrator PowerShell session."
    Exit 1
}

# --- 1. Configuration ---
$AERON_GIT_URL = 'https://github.com/real-logic/aeron.git'
$AERON_VERSION = '1.46.7'
# Source path for cloning the repository
$AERON_SRC_PATH = Join-Path (Join-Path $env:USERPROFILE 'build') 'aeron'
# Separate build directory
$BUILD_FOLDER = Join-Path (Join-Path $env:USERPROFILE 'build') 'aeron-build'
$NEEDED_EXE = 'aeronmd.exe'
$BIN_OUTPUT_FOLDER = "binaries\Release"

$SERVICE_NAME = 'AeronMediaDriver'
$SERVICE_DESC = 'Aeron C Media Driver Service'
$NSSM_EXE = 'nssm'

# Check if NSSM is accessible
if (-not (Get-Command $NSSM_EXE -ErrorAction SilentlyContinue)) {
    Write-Error "[ERROR] NSSM command '$NSSM_EXE' not found. Ensure nssm.exe is in your PATH."
    Exit 1
}

$AERON_DIR = "C:\\Temp\\aeron"  # Directory for Aeron shared memory files
$tempDir = "C:\\Temp"

$SOCKET_BUFFER_HEX = 0x400000  # 4MB in hex for registry tweaks

# Ensure C:\Temp exists for logs and shared memory
if (-not (Test-Path $tempDir)) {
    Write-Host "[INFO] Creating directory: $tempDir"
    New-Item -ItemType Directory -Path $tempDir | Out-Null
}

# Define environment variables for Aeron configuration
$envVars = @{
    "AERON_DIR" = $AERON_DIR
    "AERON_THREADING_MODE" = "DEDICATED"  # Dedicated threads for better performance
    "AERON_SOCKET_SO_RCVBUF" = "4194304"  # 4MB receive buffer
    "AERON_SOCKET_SO_SNDBUF" = "4194304"  # 4MB send buffer
    "AERON_EVENT_LOG_FILENAME" = "$tempDir\\aeron_event.log"  # Event log location
}

# --- 2. Clone or Update Aeron Repository ---
Write-Host "`n=== [1] Clone or Pull Aeron Repository ==="
if (Test-Path $AERON_SRC_PATH) {
    if (-not (Test-Path "$AERON_SRC_PATH\.git")) {
        Write-Host "[WARNING] Directory '$AERON_SRC_PATH' exists but is not a Git repository. Removing it..."
        Remove-Item -Recurse -Force $AERON_SRC_PATH
    }
}

if (-not (Test-Path "$AERON_SRC_PATH\.git")) {
    Write-Host "Cloning Aeron ($AERON_VERSION) into $AERON_SRC_PATH..."
    git clone --depth 1 --branch $AERON_VERSION $AERON_GIT_URL $AERON_SRC_PATH
    if ($LASTEXITCODE -ne 0) { Write-Error "Git clone failed."; Exit 1 }
} else {
    Write-Host "Aeron repo already present. Updating to $AERON_VERSION..."
    Push-Location $AERON_SRC_PATH
    git fetch --all --tags
    git checkout $AERON_VERSION
    git pull
    Pop-Location
}

# --- 3. Build Aeron (C++ Only, Java Disabled) ---
Write-Host "`n=== [2] Build Aeron (C++ only, Java disabled) ==="
# Ensure separate build directory exists
if (-not (Test-Path $BUILD_FOLDER)) {
    Write-Host "[INFO] Creating build output folder: $BUILD_FOLDER"
    New-Item -ItemType Directory -Path $BUILD_FOLDER | Out-Null
}

# Define the full path to aeronmd.exe in the build directory
$exeFullPath = Join-Path (Join-Path (Join-Path $BUILD_FOLDER 'binaries') 'Release') $NEEDED_EXE
Write-Host "[DEBUG] Expected aeronmd.exe path: $exeFullPath"

if ((Test-Path $exeFullPath) -and (-not $Force)) {
    Write-Host "[INFO] Build already exists. Skipping build step. Use -Force to rebuild."
} else {
    Write-Host "[INFO] Running manual CMake build..."
    Push-Location $BUILD_FOLDER

    # Configure CMake: Use Visual Studio 2022, 64-bit, disable Java, build samples, source from $AERON_SRC_PATH
    cmake -G "Visual Studio 17 2022" -A x64 -DCMAKE_BUILD_TYPE=Release -DAERON_ENABLE_JAVA=OFF -DAERON_BUILD_ARCHIVE_API=OFF -DAERON_BUILD_SAMPLES=ON -DAERON_BUILD_TOOLS=OFF $AERON_SRC_PATH

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

    if (-not (Test-Path $exeFullPath)) {
        Write-Error "[ERROR] Expected $NEEDED_EXE not found at: $exeFullPath"
        Exit 1
    }

    Write-Host "✅ Build successful. Found: $exeFullPath"
}

# --- 4. Install or Update Service via NSSM ---
Write-Host "`n=== [3] Install or Update Windows Service ==="
$servicePath = $exeFullPath  # Full path to aeronmd.exe
Write-Host "[DEBUG] Service executable path: $servicePath"

if (-not (Test-Path $servicePath)) {
    Write-Error "[ERROR] aeronmd.exe not found at path: $servicePath"
    Exit 1
}

# Ensure AERON_DIR exists for shared memory
if (-not (Test-Path $AERON_DIR)) {
    Write-Host "[INFO] Creating directory for Aeron shared memory: $AERON_DIR"
    New-Item -ItemType Directory -Path $AERON_DIR | Out-Null
}

# Convert environment variables to NSSM format
$envString = ($envVars.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join " "

$serviceObj = Get-Service -Name $SERVICE_NAME -ErrorAction SilentlyContinue

Write-Host "[DEBUG] Using NSSM: $NSSM_EXE"

if (-not $serviceObj) {
    Write-Host "[INFO] Installing service '$SERVICE_NAME' via NSSM..."
    & $NSSM_EXE install $SERVICE_NAME $servicePath
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
    & $NSSM_EXE set $SERVICE_NAME AppDirectory (Split-Path $servicePath -Parent)

    Write-Host "✅ Service installed successfully."
    Write-Host "To remove service:  nssm.exe remove $SERVICE_NAME"
} else {
    Write-Host "[INFO] Service already exists. Updating configuration..."
    & $NSSM_EXE set $SERVICE_NAME Application $servicePath
    & $NSSM_EXE set $SERVICE_NAME AppEnvironmentExtra $envString
    & $NSSM_EXE set $SERVICE_NAME AppDirectory (Split-Path $servicePath -Parent)
}

# Provide manual testing instructions
Write-Host "[INFO] To test manually in a separate terminal, run these commands:"
foreach ($var in $envVars.GetEnumerator()) {
    Write-Host "   `$env:$($var.Key) = '$($var.Value)'"
}
Write-Host "   Start-Process -NoNewWindow -FilePath '$servicePath'"
Write-Host "   # To filter logs from this run (started at $($scriptStartTime.ToString('yyyy-MM-dd HH:mm:ss'))), use:"
Write-Host "   Get-WinEvent -LogName System | Where-Object { `$_.TimeCreated -ge (Get-Date '$($scriptStartTime.ToString('yyyy-MM-dd HH:mm:ss'))') -and `$_.Message -like '*AeronMediaDriver*' } | Select-Object TimeCreated, Message | Format-List"

# Start the service if not running
$serviceObj = Get-Service -Name $SERVICE_NAME
if ($serviceObj.Status -ne 'Running') {
    Write-Host "[INFO] Logs:"
    Write-Host "  - Stdout: $tempDir\\aeron_stdout.log"
    Write-Host "  - Stderr: $tempDir\\aeron_stderr.log"
    Write-Host "  - Event Log: $tempDir\\aeron_event.log"
    Write-Host "[INFO] Starting service..."
    Write-Host "[DEBUG] Starting service with: $NSSM_EXE start $SERVICE_NAME"
    & $NSSM_EXE start $SERVICE_NAME

    # Wait up to 30 seconds for the service to start
    $timeoutSeconds = 30
    $intervalMilliseconds = 1000
    $elapsed = 0

    while ($elapsed -lt $timeoutSeconds * 1000) {
        Start-Sleep -Milliseconds $intervalMilliseconds
        $serviceObj.Refresh()
        if ($serviceObj.Status -eq 'Running') {
            break
        }
        $elapsed += $intervalMilliseconds
    }

    if ($serviceObj.Status -ne 'Running') {
        Write-Error "[ERROR] Failed to start service '$SERVICE_NAME' within $timeoutSeconds seconds."
        Write-Host "[DEBUG] Dumping last 10 lines of stderr log:"
        Get-Content "$tempDir\\aeron_stderr.log" -Tail 10 -ErrorAction SilentlyContinue
        Exit 1
    }
    Write-Host "✅ Service started."

    # Show recent System log events
    Write-Host "[INFO] Checking recent System log events for AeronMediaDriver since script start ($scriptStartTime)..."
    Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=$scriptStartTime} | Where-Object { $_.Message -like "*AeronMediaDriver*" } | Select-Object TimeCreated, Message | Format-List
} else {
    Write-Host "[INFO] Service is already running."
}

# --- 5. Registry Tuning ---
Write-Host "`n=== [4] Apply Windows Registry Tweaks (UDP Performance) ==="
reg add "HKLM\SYSTEM\CurrentControlSet\Services\Afd\Parameters" /v DefaultSendWindow /t REG_DWORD /d $SOCKET_BUFFER_HEX /f | Out-Null
reg add "HKLM\SYSTEM\CurrentControlSet\Services\Afd\Parameters" /v DefaultReceiveWindow /t REG_DWORD /d $SOCKET_BUFFER_HEX /f | Out-Null
reg add "HKLM\SYSTEM\CurrentControlSet\Services\Afd\Parameters" /v FastSendDatagramThreshold /t REG_DWORD /d 1500 /f | Out-Null
reg add "HKLM\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters" /v MaximumReassemblyHeaders /t REG_DWORD /d 0xFFFF /f | Out-Null
reg add "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile" /v NetworkThrottlingIndex /t REG_DWORD /d 0xFFFFFFFF /f | Out-Null

Write-Host "`n[INFO] Registry tuning applied. A reboot may be required."

# --- Done ---
Write-Host "`n=== ✅ Setup Complete ==="
Write-Host "Aeron source code:  '$AERON_SRC_PATH'"
Write-Host "Build directory:    '$BUILD_FOLDER'"
Write-Host "Built artifacts:    '$exeFullPath'"
Write-Host "Service name:       '$SERVICE_NAME'"
Write-Host "To check status:    sc.exe query $SERVICE_NAME"
Write-Host "To stop service:    sc.exe stop $SERVICE_NAME"
Write-Host "To remove service:  nssm.exe remove $SERVICE_NAME"
Write-Host "To check logs:      Get-Content $tempDir\\aeron_stdout.log -Tail 20 -Wait"
Write-Host "                   Get-Content $tempDir\\aeron_stderr.log -Tail 20 -Wait"
Write-Host "                   Get-Content $tempDir\\aeron_event.log -Tail 20 -Wait"
Write-Host "To test manually:   Start-Process -NoNewWindow -FilePath '$servicePath'"