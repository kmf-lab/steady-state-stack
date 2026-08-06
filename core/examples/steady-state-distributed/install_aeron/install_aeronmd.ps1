<#
.SYNOPSIS
    Installs Aeron C++ Media Driver on Windows 11 with detailed logging for debugging runtime issues.

.DESCRIPTION
    This script automates the installation of prerequisites (Chocolatey, Visual Studio with C++ workload, CMake, Git, NSSM),
    builds the C++ Media Driver and example projects, and sets up the driver as a Windows service.
    Updated to ensure the target user has "Log on as a service" rights and appropriate directory permissions.

.PARAMETER TargetUser
    The Windows user account to run the AeronMediaDriver service as. Defaults to the current user.

.NOTES
    - Must be run as Administrator.
    - Designed for Windows 11.
    - Installs Aeron version 1.48.0.
    - Logs are saved to C:\Temp\install_aeronmd.log and service logs in C:\Temp.
#>

param(
    [string]$TargetUser = "$env:USERDOMAIN\$env:USERNAME"
)

# --- Initialize Logging ---
$global:setupLogFile = "C:\Temp\install_aeronmd.log"
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
    Add-Content -Path $global:setupLogFile -Value $logEntry -ErrorAction SilentlyContinue
}

function Fail-And-Exit {
    param([string]$msg)
    Log-Message $msg "ERROR"
    Write-Host "`nERROR: $msg"
    Write-Host "See the full log at $global:setupLogFile"
    Stop-Transcript | Out-Null
    Exit 1
}

# Function to grant "Log on as a service" right
function Grant-LogOnAsService {
    param([string]$User)

    # Parse domain and username
    if ($User -match "^(.+)\\(.+)$") {
        $domain = $matches[1]
        $username = $matches[2]
    } else {
        $domain = $env:COMPUTERNAME
        $username = $User
    }

    # Get SID
    try {
        $account = New-Object System.Security.Principal.NTAccount($domain, $username)
        $sid = $account.Translate([System.Security.Principal.SecurityIdentifier]).Value
    } catch {
        Fail-And-Exit "Could not find user $User. Please ensure the account exists."
    }

    # Export security settings
    $exportFile = "$env:TEMP\secpol.cfg"
    secedit /export /cfg $exportFile /quiet

    # Read the file
    $secpol = Get-Content $exportFile

    # Find the SeServiceLogonRight line
    $line = $secpol | Where-Object { $_ -like "SeServiceLogonRight*" }

    if ($line) {
        if ($line -notlike "*$sid*") {
            Log-Message "Granting 'Log on as a service' right to $User"
            $newLine = $line + ",*$sid"
            $secpol = $secpol -replace [regex]::Escape($line), $newLine
            $secpol | Set-Content $exportFile
            secedit /configure /db $env:windir\security\local.sdb /cfg $exportFile /areas USER_RIGHTS
            Remove-Item $exportFile
            gpupdate /force
            Start-Sleep -Seconds 5
        } else {
            Log-Message "$User already has 'Log on as a service' right"
        }
    } else {
        Log-Message "Adding 'Log on as a service' right for $User"
        $secpol += "SeServiceLogonRight = *$sid"
        $secpol | Set-Content $exportFile
        secedit /configure /db $env:windir\security\local.sdb /cfg $exportFile /areas USER_RIGHTS
        Remove-Item $exportFile
        gpupdate /force
        Start-Sleep -Seconds 5
    }
}

try {
    Log-Message "Starting Aeron installation script."
    Log-Message "Target user for service: $TargetUser"

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
        @{Name="Visual Studio 2022 Community"; Package="visualstudio2022community"; Params="--add Microsoft.VisualStudio.Workload.NativeDesktop --includeRecommended"},
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

    # --- 4. Check CMake version compatibility ---
    $cmakeVersionMatch = (& cmake --version | Select-String -Pattern "\d+\.\d+\.\d+" | Select-Object -First 1)
    if ($cmakeVersionMatch) {
        $cmakeVersion = $cmakeVersionMatch.Matches[0].Value
        if ([version]$cmakeVersion -lt [version]"3.22.0") {
            Log-Message "WARNING: CMake version $cmakeVersion may be too old for Aeron 1.48.0." "WARNING"
        }
    } else {
        Log-Message "Could not determine CMake version!" "WARNING"
    }

    # --- 5. Ensure C:\Temp exists and set up C:\Temp\aeron ---
    $tempDir = "C:\Temp"
    Log-Message "Checking for directory: $tempDir"
    if (-not (Test-Path $tempDir)) {
        Log-Message "Creating directory: $tempDir"
        New-Item -ItemType Directory -Path $tempDir | Out-Null
        if (-not (Test-Path $tempDir)) {
            Fail-And-Exit "Failed to create directory: $tempDir"
        }
        Log-Message "Directory created: $tempDir"
    } else {
        Log-Message "Directory already exists: $tempDir"
    }

    # Ensure C:\Temp\aeron exists with permissions for TargetUser
    $aeronDir = "$tempDir\aeron"
    if (-not (Test-Path $aeronDir)) {
        Log-Message "Creating directory: $aeronDir"
        New-Item -ItemType Directory -Path $aeronDir | Out-Null
    }
    Log-Message "Setting permissions for $TargetUser on $aeronDir"
    $acl = Get-Acl $aeronDir
    $rule = New-Object System.Security.AccessControl.FileSystemAccessRule($TargetUser, "FullControl", "ContainerInherit,ObjectInherit", "None", "Allow")
    $acl.SetAccessRule($rule)
    Set-Acl $aeronDir $acl

    # --- 6. Clean Previous Aeron Source and Build Folders ---
    $AERON_VERSION = '1.48.0'
    $AERON_SRC_PATH = Join-Path (Join-Path $env:USERPROFILE 'build') 'aeron'
    $BUILD_ROOT = Join-Path $env:USERPROFILE 'build'
    Log-Message "Deleting previous Aeron source and build directories..."
    Remove-Item -Recurse -Force $AERON_SRC_PATH -ErrorAction SilentlyContinue
    Get-ChildItem -Path $BUILD_ROOT -Filter "aeron-build*" | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue

    # --- 7. Clone Aeron Repository ---
    Log-Message "Cloning Aeron ($AERON_VERSION) into $AERON_SRC_PATH..."
    git clone --depth 1 --branch $AERON_VERSION https://github.com/real-logic/aeron.git $AERON_SRC_PATH
    if ($LASTEXITCODE -ne 0) {
        Fail-And-Exit "Git clone failed."
    }
    Log-Message "Aeron repository cloned successfully."

    # --- 8. Build Aeron ---
    $BUILD_FOLDER = Join-Path $BUILD_ROOT ("aeron-build-" + (Get-Date -Format "yyyyMMdd-HHmmss"))
    Log-Message "Creating new build folder: $BUILD_FOLDER"
    New-Item -ItemType Directory -Path $BUILD_FOLDER | Out-Null

    Push-Location $BUILD_FOLDER

    $cmakeLog = Join-Path $tempDir "cmake_configure.log"
    Log-Message "Configuring CMake (log: $cmakeLog)..."
    cmake -G "Visual Studio 17 2022" -A x64 `
        -DCMAKE_BUILD_TYPE=Release `
        -DAERON_ENABLE_JAVA=OFF `
        -DAERON_BUILD_SAMPLES=ON `
        -DAERON_BUILD_TOOLS=OFF `
        $AERON_SRC_PATH --trace-expand | Tee-Object -FilePath $cmakeLog -Append
    if ($LASTEXITCODE -ne 0) {
        Pop-Location
        Fail-And-Exit "CMake configuration failed. See log: $cmakeLog"
    }
    Log-Message "CMake configuration successful."

    $buildLog = Join-Path $tempDir "cmake_build.log"
    Log-Message "Building aeronmd.exe (log: $buildLog)..."
    cmake --build . --config Release --target aeronmd -- /v:diag 2>&1 | Out-File -FilePath $buildLog -Append
    if ($LASTEXITCODE -ne 0) {
        Pop-Location
        Fail-And-Exit "CMake build failed. See log: $buildLog"
    }
    Log-Message "Aeron build successful."

    Log-Message "Building basic_publisher, basic_subscriber, streaming_publisher (log: $buildLog)..."
    cmake --build . --config Release --target basic_publisher basic_subscriber streaming_publisher -- /v:diag 2>&1 | Out-File -FilePath $buildLog -Append
    if ($LASTEXITCODE -ne 0) {
        Pop-Location
        Fail-And-Exit "CMake build failed for examples. See log: $buildLog"
    }
    Log-Message "basic_publisher, basic_subscriber, streaming_publisher build successful."

    Pop-Location

    # --- 9. Find aeronmd.exe ---
    $AERONMD_EXE = Get-ChildItem -Path $BUILD_FOLDER -Recurse -Filter "aeronmd.exe" | Select-Object -First 1
    if (-not $AERONMD_EXE) {
        Fail-And-Exit "Could not find aeronmd.exe after build! See $buildLog"
    }
    Log-Message "Found aeronmd.exe at $($AERONMD_EXE.FullName)"

    # --- 10. Ensure TargetUser has "Log on as a service" right ---
    Log-Message "Ensuring $TargetUser has 'Log on as a service' right..."
    Grant-LogOnAsService -User $TargetUser

    # --- 11. Install or Update Service via NSSM ---
    $SERVICE_NAME = 'AeronMediaDriver'
    $SERVICE_DESC = 'Aeron C Media Driver Service'
    $NSSM_EXE = 'nssm'

    if (Get-Service -Name $SERVICE_NAME -ErrorAction SilentlyContinue) {
        Log-Message "Stopping and removing existing service: $SERVICE_NAME"
        & $NSSM_EXE stop $SERVICE_NAME
        Start-Sleep -Seconds 2
        & $NSSM_EXE remove $SERVICE_NAME confirm
        Start-Sleep -Seconds 2
    }

    Log-Message "Installing Aeron Media Driver service..."
    & $NSSM_EXE install $SERVICE_NAME $AERONMD_EXE.FullName
    & $NSSM_EXE set $SERVICE_NAME Description "$SERVICE_DESC"
    & $NSSM_EXE set $SERVICE_NAME AppDirectory $BUILD_FOLDER
    & $NSSM_EXE set $SERVICE_NAME AppStdout "$tempDir\AeronService.log"
    & $NSSM_EXE set $SERVICE_NAME AppStderr "$tempDir\AeronService.err.log"
    & $NSSM_EXE set $SERVICE_NAME Start SERVICE_AUTO_START
	& $NSSM_EXE set $SERVICE_NAME AppPriority ABOVE_NORMAL_PRIORITY_CLASS
    & $NSSM_EXE set $SERVICE_NAME ObjectName $TargetUser  # Set service to run as TargetUser

    # Set environment variables for the service
    $nssmEnvVars = @(
        "AERON_DIR=C:\Temp\aeron",
        "AERON_THREADING_MODE=DEDICATED",
        "AERON_MEMORY_MAPPED_FILE_MODE=PERFORMANCE",
        "AERON_SOCKET_SO_RCVBUF=16777216",
        "AERON_SOCKET_SO_SNDBUF=16777216",
        "AERON_CONDUCTOR_IDLE_STRATEGY=backoff",
        "AERON_SENDER_IDLE_STRATEGY=backoff",
        "AERON_RECEIVER_IDLE_STRATEGY=backoff",
        "AERON_RCV_INITIAL_WINDOW_LENGTH=16777216",
		"AERON_TERM_BUFFER_LENGTH=67108864",
        "AERON_EVENT_LOG_FILENAME=$tempDir\aeron_event.log"
    )
    $nssmEnvString = $nssmEnvVars -join "`r`n"
    & $NSSM_EXE set $SERVICE_NAME AppEnvironmentExtra $nssmEnvString 2>&1 | Out-Null



    # Start service
    Write-Host "Starting Aeron Media Driver service..."
    & $NSSM_EXE start $SERVICE_NAME

    Write-Host "Aeron Media Driver installation and service setup complete."
    Write-Host "`nAeron Media Driver is installed and running as a Windows service ($SERVICE_NAME)."
    Write-Host "Check logs in $tempDir for details."
    Write-Host "Full setup log: $global:setupLogFile"
    Write-Host "Check events:  eventvwr.msc"

    # --- 12. Show Example Usage ---
    $BASIC_PUB = Get-ChildItem -Path $BUILD_FOLDER -Recurse -Filter "basic_publisher.exe" | Select-Object -First 1
    $BASIC_SUB = Get-ChildItem -Path $BUILD_FOLDER -Recurse -Filter "basic_subscriber.exe" | Select-Object -First 1
    $STREAM_PUB = Get-ChildItem -Path $BUILD_FOLDER -Recurse -Filter "streaming_publisher.exe" | Select-Object -First 1

    Write-Host ""
    Write-Host "=== Aeron Example Binaries Built ==="
    if ($BASIC_PUB) { Write-Host "basic_publisher.exe: $($BASIC_PUB.FullName)" }
    if ($BASIC_SUB) { Write-Host "basic_subscriber.exe: $($BASIC_SUB.FullName)" }
    if ($STREAM_PUB) { Write-Host "streaming_publisher.exe: $($STREAM_PUB.FullName)" }
    Write-Host ""

    if ($BASIC_PUB -and $BASIC_SUB) {
        Write-Host ""
        Write-Host "=== Example: Test the Aeron Media Driver Service ==="
        Write-Host "Before running the test binaries, set the environment variable in each terminal:"
        Write-Host '$env:AERON_DIR = "C:\Temp\aeron"'
        Write-Host ""
        Write-Host "# Terminal 1: Start the subscriber"
        Write-Host "$($BASIC_SUB.FullName) -c aeron:udp?endpoint=localhost:40123"
        Write-Host ""
        Write-Host "# Terminal 2: Start the publisher"
        Write-Host "$($BASIC_PUB.FullName) -c aeron:udp?endpoint=localhost:40123"
        Write-Host ""
        Write-Host "You should see messages being sent and received via the Aeron Media Driver service."
        Write-Host ""
    }
    if ($STREAM_PUB) {
        Write-Host "# Example: Streaming publisher"
        Write-Host "$($STREAM_PUB.FullName) -c aeron:udp?endpoint=localhost:40123 -m 1000 -L 8"
    }
    Write-Host ""

    Stop-Transcript | Out-Null
}
catch {
    Write-Host "`nERROR: $($_.Exception.Message)"
    Write-Host "See the full log at $global:setupLogFile"
    Stop-Transcript | Out-Null
    Exit 1
}