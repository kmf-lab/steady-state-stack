<#
.SYNOPSIS
    Installs Aeron C++ Media Driver on Windows 11 with detailed logging for debugging runtime issues.

.DESCRIPTION
    This script automates the installation of prerequisites (Chocolatey, Visual Studio with C++ workload, CMake, Git, NSSM),
    clones the Aeron repository, builds the C++ Media Driver, sets it up as a Windows service, and applies performance tweaks.
    Extensive logging is added to track every step and capture outputs for post-installation debugging.

.NOTES
    - Must be run as Administrator.
    - Designed for Windows 11.
    - Installs Aeron version 1.46.7.
    - Logs are saved to C:\Temp\AeronInstallLog.txt and service logs in C:\Temp.
#>

# --- Initialize Logging ---
$logFile = "C:\Temp\AeronInstallLog.txt"

function Log-Message {
    param (
        [string]$message,
        [string]$level = "INFO"
    )
    $timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    $logEntry = "[$timestamp] [$level] $message"
    Write-Host $logEntry
    Add-Content -Path $logFile -Value $logEntry -ErrorAction SilentlyContinue
}

Log-Message "Starting Aeron installation script."

# --- 0. Ensure Running as Administrator ---
Log-Message "Checking if script is running as Administrator..."
if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Log-Message "Script is not running as Administrator. Attempting to relaunch..." -level "WARNING"
    Start-Process powershell -Verb RunAs -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`""
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
    iex ((New-Object System.Net.WebClient).DownloadString('https://chocolatey.org/install.ps1')) 2>&1 | Out-File -Append -FilePath $logFile
    if ($LASTEXITCODE -ne 0) {
        Log-Message "Chocolatey installation failed." -level "ERROR"
        Exit 1
    }
    # Refresh environment variables
    $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")
    Log-Message "Chocolatey installed successfully."
} else {
    Log-Message "Chocolatey is already installed."
}

# --- 2. Install Required Tools via Chocolatey ---
Log-Message "Installing required tools via Chocolatey..."
$toolsToInstall = @(
    @{Name="Visual Studio 2022 Community"; Package="visualstudio2022community"; Params="--add Microsoft.VisualStudio.Workload.NativeDesktop --includeRecommended"},
    @{Name="CMake"; Package="cmake"},
    @{Name="Git"; Package="git"},
    @{Name="NSSM"; Package="nssm"}
)

foreach ($tool in $toolsToInstall) {
    Log-Message "Installing $($tool.Name)..."
    $installactor = "choco install $($tool.Package) -y"
    if ($tool.Params) {
        $installactor += " --package-parameters `"$($tool.Params)`""
    }
    Invoke-Expression $installactor 2>&1 | Out-File -Append -FilePath $logFile
    if ($LASTEXITCODE -ne 0) {
        Log-Message "Failed to install $($tool.Name)." -level "ERROR"
        Exit 1
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
        Log-Message "$($tool.Name) is not installed or not in PATH." -level "ERROR"
        Exit 1
    } else {
        Log-Message "$($tool.Name) is installed."
    }
}

# --- 4. Ensure C:\Temp exists for logs and shared memory ---
$tempDir = "C:\Temp"
Log-Message "Checking for directory: $tempDir"
if (-not (Test-Path $tempDir)) {
    Log-Message "Creating directory: $tempDir"
    New-Item -ItemType Directory -Path $tempDir | Out-Null
    if (-not (Test-Path $tempDir)) {
        Log-Message "Failed to create directory: $tempDir" -level "ERROR"
        Exit 1
    }
    Log-Message "Directory created: $tempDir"
} else {
    Log-Message "Directory already exists: $tempDir"
}

# --- 5. Clone or Update Aeron Repository ---
Log-Message "=== [1] Clone or Pull Aeron Repository ==="
$AERON_VERSION = '1.46.7'
$AERON_SRC_PATH = Join-Path (Join-Path $env:USERPROFILE 'build') 'aeron'

Log-Message "Checking for Aeron repository at $AERON_SRC_PATH"
if (-not (Test-Path "$AERON_SRC_PATH\.git")) {
    Log-Message "Cloning Aeron ($AERON_VERSION) into $AERON_SRC_PATH..."
    git clone --depth 1 --branch $AERON_VERSION https://github.com/real-logic/aeron.git $AERON_SRC_PATH 2>&1 | Out-File -Append -FilePath $logFile
    if ($LASTEXITCODE -ne 0) {
        Log-Message "Git clone failed." -level "ERROR"
        Exit 1
    }
    Log-Message "Aeron repository cloned successfully."
} else {
    Log-Message "Aeron repository already present. Updating to $AERON_VERSION..."
    Push-Location $AERON_SRC_PATH
    git fetch --all --tags 2>&1 | Out-File -Append -FilePath $logFile
    git checkout $AERON_VERSION 2>&1 | Out-File -Append -FilePath $logFile
    git pull 2>&1 | Out-File -Append -FilePath $logFile
    Pop-Location
    if ($LASTEXITCODE -ne 0) {
        Log-Message "Failed to update Aeron repository." -level "ERROR"
        Exit 1
    }
    Log-Message "Aeron repository updated to $AERON_VERSION."
}

# --- 6. Build Aeron (C++ Only, Java Disabled) ---
Log-Message "=== [2] Build Aeron (C++ only, Java disabled) ==="
$BUILD_FOLDER = Join-Path (Join-Path $env:USERPROFILE 'build') 'aeron-build'
Log-Message "Checking for build folder: $BUILD_FOLDER"
if (-not (Test-Path $BUILD_FOLDER)) {
    Log-Message "Creating build folder: $BUILD_FOLDER"
    New-Item -ItemType Directory -Path $BUILD_FOLDER | Out-Null
    if (-not (Test-Path $BUILD_FOLDER)) {
        Log-Message "Failed to create build folder: $BUILD_FOLDER" -level "ERROR"
        Exit 1
    }
    Log-Message "Build folder created: $BUILD_FOLDER"
} else {
    Log-Message "Build folder already exists: $BUILD_FOLDER"
}

Push-Location $BUILD_FOLDER
Log-Message "Configuring CMake..."
cmake -G "Visual Studio 17 2022" -A x64 -DCMAKE_BUILD_TYPE=Release -DAERON_ENABLE_JAVA=OFF -DAERON_BUILD_SAMPLES=ON -DAERON_BUILD_TOOLS=OFF $AERON_SRC_PATH 2>&1 | Out-File -Append -FilePath $logFile
if ($LASTEXITCODE -ne 0) {
    Log-Message "CMake configuration failed." -level "ERROR"
    Exit 1
}
Log-Message "CMake configuration successful."

Log-Message "Building Aeron targets..."
cmake --build . --config Release --target aeronmd basic_publisher basic_subscriber streaming_publisher 2>&1 | Out-File -Append -FilePath $logFile
if ($LASTEXITCODE -ne 0) {
    Log-Message "CMake build failed." -level "ERROR"
    Exit 1
}
Log-Message "Aeron build successful."
Pop-Location

# --- 7. Install or Update Service via NSSM ---
Log-Message "=== [3] Install or Update Windows Service ==="
$SERVICE_NAME = 'AeronMediaDriver'
$SERVICE_DESC = 'Aeron C Media Driver Service'
$NSSM_EXE = 'nssm'
$exeFullPath = Join-Path (Join-Path (Join-Path $BUILD_FOLDER 'binaries') 'Release') 'aeronmd.exe'
$basicPublisherPath = Join-Path (Join-Path (Join-Path $BUILD_FOLDER 'binaries') 'Release') 'basic_publisher.exe'
$basicSubscriberPath = Join-Path (Join-Path (Join-Path $BUILD_FOLDER 'binaries') 'Release') 'basic_subscriber.exe'
$streamingPublisherPath = Join-Path (Join-Path (Join-Path $BUILD_FOLDER 'binaries') 'Release') 'streaming_publisher.exe'

Log-Message "Checking for aeronmd.exe at $exeFullPath"
if (-not (Test-Path $exeFullPath)) {
    Log-Message "aeronmd.exe not found at $exeFullPath" -level "ERROR"
    Exit 1
}
Log-Message "aeronmd.exe found."

$AERON_DIR = "C:\Temp\aeron"
Log-Message "Checking for Aeron shared memory directory: $AERON_DIR"
if (-not (Test-Path $AERON_DIR)) {
    Log-Message "Creating directory: $AERON_DIR"
    New-Item -ItemType Directory -Path $AERON_DIR | Out-Null
    if (-not (Test-Path $AERON_DIR)) {
        Log-Message "Failed to create directory: $AERON_DIR" -level "ERROR"
        Exit 1
    }
    Log-Message "Directory created: $AERON_DIR"
} else {
    Log-Message "Directory already exists: $AERON_DIR"
}


Log-Message "Checking for service: $SERVICE_NAME"
$serviceObj = Get-Service -Name $SERVICE_NAME -ErrorAction SilentlyContinue
if (-not $serviceObj) {
    Log-Message "Installing service '$SERVICE_NAME' via NSSM..."
    & $NSSM_EXE install $SERVICE_NAME $exeFullPath 2>&1 | Out-File -Append -FilePath $logFile
    if ($LASTEXITCODE -ne 0) {
        Log-Message "NSSM install failed." -level "ERROR"
        Exit 1
    }
    & $NSSM_EXE set $SERVICE_NAME Description $SERVICE_DESC 2>&1 | Out-File -Append -FilePath $logFile
    & $NSSM_EXE set $SERVICE_NAME Start SERVICE_AUTO_START 2>&1 | Out-File -Append -FilePath $logFile
    & $NSSM_EXE set $SERVICE_NAME AppThrottle 0 2>&1 | Out-File -Append -FilePath $logFile
	& $NSSM_EXE set $SERVICE_NAME AppEnvironmentExtra `
		"AERON_DIR=$AERON_DIR" `
		"AERON_THREADING_MODE=DEDICATED" `
		"AERON_SOCKET_SO_RCVBUF=4194304" `
		"AERON_SOCKET_SO_SNDBUF=4194304" `
		"AERON_EVENT_LOG_FILENAME=$tempDir\\aeron_event.log"
    & $NSSM_EXE set $SERVICE_NAME AppRestartDelay 5000 2>&1 | Out-File -Append -FilePath $logFile
    & $NSSM_EXE set $SERVICE_NAME AppStdout "$tempDir\\aeron_stdout.log" 2>&1 | Out-File -Append -FilePath $logFile
    & $NSSM_EXE set $SERVICE_NAME AppStderr "$tempDir\\aeron_stderr.log" 2>&1 | Out-File -Append -FilePath $logFile
    & $NSSM_EXE set $SERVICE_NAME AppRotateFiles 1 2>&1 | Out-File -Append -FilePath $logFile
    & $NSSM_EXE set $SERVICE_NAME AppDirectory (Split-Path $exeFullPath -Parent) 2>&1 | Out-File -Append -FilePath $logFile
    Log-Message "Service installed successfully."
} else {
    Log-Message "Service already exists. Updating configuration..."
    & $NSSM_EXE set $SERVICE_NAME Application $exeFullPath 2>&1 | Out-File -Append -FilePath $logFile
    & $NSSM_EXE set $SERVICE_NAME AppEnvironmentExtra "$envString" 2>&1 | Out-File -Append -FilePath $logFile
    & $NSSM_EXE set $SERVICE_NAME AppDirectory (Split-Path $exeFullPath -Parent) 2>&1 | Out-File -Append -FilePath $logFile
    Log-Message "Service configuration updated."
}

# Apply the configuration by restarting or starting the service
$serviceObj = Get-Service -Name $SERVICE_NAME -ErrorAction SilentlyContinue
if ($serviceObj) {
    if ($serviceObj.Status -eq 'Running') {
        Log-Message "Restarting service '$SERVICE_NAME' to apply new environment variables..."
        & $NSSM_EXE restart $SERVICE_NAME 2>&1 | Out-File -Append -FilePath $logFile
        if ($LASTEXITCODE -ne 0) {
            Log-Message "Failed to restart service '$SERVICE_NAME'." -level "ERROR"
            Exit 1
        }
        Log-Message "Service restarted successfully."
    } else {
        Log-Message "Starting service '$SERVICE_NAME'..."
        & $NSSM_EXE start $SERVICE_NAME 2>&1 | Out-File -Append -FilePath $logFile
        if ($LASTEXITCODE -ne 0) {
            Log-Message "Failed to start service '$SERVICE_NAME'." -level "ERROR"
            Exit 1
        }
        Log-Message "Service started successfully."
    }
}

# Verify service is running
Start-Sleep -Seconds 5
$serviceObj = Get-Service -Name $SERVICE_NAME
if ($serviceObj.Status -ne 'Running') {
    Log-Message "Service '$SERVICE_NAME' failed to remain running." -level "ERROR"
    if (Test-Path "$tempDir\\aeron_stderr.log") {
        Log-Message "Last 5 lines of stderr log:"
        Get-Content "$tempDir\\aeron_stderr.log" -Tail 5 | ForEach-Object { Log-Message $_ }
    }
    Exit 1
}
Log-Message "Service '$SERVICE_NAME' is running with updated environment variables."

# Log all service configuration details
Log-Message "=== Service Configuration Details ==="
$serviceParams = @("Application", "AppDirectory", "AppEnvironmentExtra", "Start", "Description", "AppStdout", "AppStderr", "AppRotateFiles", "AppExit", "AppRestartDelay")
foreach ($param in $serviceParams) {
    $value = & $NSSM_EXE get $SERVICE_NAME $param
    Log-Message "${param}: ${value}"
}

# --- 8. Apply Windows Registry Tweaks for UDP Performance ---
Log-Message "=== [4] Apply Windows Registry Tweaks (UDP Performance) ==="
$SOCKET_BUFFER_HEX = 0x400000
reg add "HKLM\SYSTEM\CurrentControlSet\Services\Afd\Parameters" /v DefaultSendWindow /t REG_DWORD /d $SOCKET_BUFFER_HEX /f | Out-Null
if ($LASTEXITCODE -eq 0) {
    Log-Message "Set DefaultSendWindow to $SOCKET_BUFFER_HEX"
} else {
    Log-Message "Failed to set DefaultSendWindow" -level "WARNING"
}
reg add "HKLM\SYSTEM\CurrentControlSet\Services\Afd\Parameters" /v DefaultReceiveWindow /t REG_DWORD /d $SOCKET_BUFFER_HEX /f | Out-Null
if ($LASTEXITCODE -eq 0) {
    Log-Message "Set DefaultReceiveWindow to $SOCKET_BUFFER_HEX"
} else {
    Log-Message "Failed to set DefaultReceiveWindow" -level "WARNING"
}
reg add "HKLM\SYSTEM\CurrentControlSet\Services\Afd\Parameters" /v FastSendDatagramThreshold /t REG_DWORD /d 1500 /f | Out-Null
if ($LASTEXITCODE -eq 0) {
    Log-Message "Set FastSendDatagramThreshold to 1500"
} else {
    Log-Message "Failed to set FastSendDatagramThreshold" -level "WARNING"
}
reg add "HKLM\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters" /v MaximumReassemblyHeaders /t REG_DWORD /d 0xFFFF /f | Out-Null
if ($LASTEXITCODE -eq 0) {
    Log-Message "Set MaximumReassemblyHeaders to 0xFFFF"
} else {
    Log-Message "Failed to set MaximumReassemblyHeaders" -level "WARNING"
}
reg add "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile" /v NetworkThrottlingIndex /t REG_DWORD /d 0xFFFFFFFF /f | Out-Null
if ($LASTEXITCODE -eq 0) {
    Log-Message "Set NetworkThrottlingIndex to 0xFFFFFFFF"
} else {
    Log-Message "Failed to set NetworkThrottlingIndex" -level "WARNING"
}
Log-Message "Registry tweaks applied. A reboot may be required for changes to take effect."

# --- 9. Final Instructions ---
Log-Message "Setup complete."
Log-Message "Aeron source code: '$AERON_SRC_PATH'"
Log-Message "Build directory: '$BUILD_FOLDER'"
Log-Message "Service name: '$SERVICE_NAME'"
Log-Message "To check service status: sc.exe query $SERVICE_NAME"
Log-Message "To stop service: sc.exe stop $SERVICE_NAME"
Log-Message "To remove service: nssm.exe remove $SERVICE_NAME"
Log-Message "To see service: nssm.exe dump $SERVICE_NAME"

Log-Message "Check installation log: Get-Content $logFile -Tail 20 -Wait"
Log-Message "Check service logs:"
Log-Message "  Get-Content $tempDir\\aeron_stdout.log -Tail 20 -Wait"
Log-Message "  Get-Content $tempDir\\aeron_stderr.log -Tail 20 -Wait"
Log-Message "  Get-Content $tempDir\\aeron_event.log -Tail 20 -Wait"
Log-Message "If service fails to start, check Windows Event Log: Get-EventLog -LogName Application -Source NSSM -Newest 10"

# Log manual testing instructions as a single block, including sample binaries
$manualTestInstructions = "To manually test the media driver and sample applications, run the following in PowerShell:`n`n"
$manualTestInstructions += "# Set environment variables`n"
$manualTestInstructions += "`$env:AERON_DIR = '$AERON_DIR'`n"
$manualTestInstructions += "`$env:AERON_THREADING_MODE = 'DEDICATED'`n"
$manualTestInstructions += "`$env:AERON_SOCKET_SO_RCVBUF = '4194304'`n"
$manualTestInstructions += "`$env:AERON_SOCKET_SO_SNDBUF = '4194304'`n"
$manualTestInstructions += "`$env:AERON_EVENT_LOG_FILENAME = '$tempDir\\aeron_event.log'`n`n"
$manualTestInstructions += "# In one PowerShell window, run the media driver:`n"
$manualTestInstructions += "& '$exeFullPath'`n`n"
$manualTestInstructions += "# In a second PowerShell window, run the basic publisher:`n"
$manualTestInstructions += "& '$basicPublisherPath'`n`n"
$manualTestInstructions += "# In a third PowerShell window, run the basic subscriber:`n"
$manualTestInstructions += "& '$basicSubscriberPath'`n`n"
$manualTestInstructions += "# In a fourth PowerShell window, run the streaming publisher:`n"
$manualTestInstructions += "& '$streamingPublisherPath'"
Log-Message $manualTestInstructions

nssm.exe dump $SERVICE_NAME

$env:AERON_DIR = 'C:\Temp\aeron'

Log-Message "  sc.exe query $SERVICE_NAME"
sc.exe query AeronMediaDriver