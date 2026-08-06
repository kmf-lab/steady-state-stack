# Aeron Media Driver Service Setup Guide (Windows)

This guide covers installing, verifying, and uninstalling the Aeron C++ Media Driver Service on Windows using the `install_aeronmd.ps1` script. It is designed for use with applications built on the **steady\_state** framework in Rust.

---

## Prerequisites

1. **Operating System:** Windows 11 (or Windows Server 2022).
2. **Administrator Privileges:** You must run PowerShell as Administrator.
3. **PowerShell Execution Policy:** The script will set `Bypass` for its own execution; no additional steps are required.
4. **Internet Access:** Required to clone the Aeron repository and download packages via Chocolatey.

---

## 1. Installation

1. **Open PowerShell as Administrator**

    * Right‑click on the PowerShell icon and choose **Run as Administrator**.

2. **Navigate to the Script Directory**

   ```powershell
   cd C:\Path\To\aeron_media_driver
   ```

3. **Unblock and Execute the Script**

   ```powershell
   Unblock-File .\install_aeronmd.ps1
   .\install_aeronmd.ps1
   ```

    * The script will:

        * Install or verify **Chocolatey**.
        * Install Visual Studio 2022 Community (C++ workload), CMake, Git, and NSSM.
        * Clone **Aeron v1.48.0** into `C:\Users\<YourUser>\build\aeron` (clean each run).
        * Configure and build the C++ Media Driver (`aeronmd.exe`) and samples.
        * Install or update the **AeronMediaDriver** Windows service via NSSM.
        * Apply UDP performance registry tweaks.
    * **Logs** are written to:

        * `C:\Temp\AeronInstallLog.txt`
        * `C:\Temp\cmake_configure.log`
        * `C:\Temp\cmake_build.log`

4. **Verify Service is Running**

   ```powershell
   Get-Service -Name AeronMediaDriver
   sc.exe query AeronMediaDriver
   ```

    * Check logs if the service failed to start:

      ```powershell
      Get-Content C:\Temp\aeron_stderr.log -Tail 20 -Wait
      Get-Content C:\Temp\aeron_event.log -Tail 20 -Wait
      ```

---

## 2. Uninstallation

1. **Stop and Remove the Service**

   ```powershell
   sc.exe stop AeronMediaDriver
   nssm.exe remove AeronMediaDriver confirm
   ```
2. **Clean Up Build and Source Folders** (optional)

   ```powershell
   Remove-Item -Recurse -Force "$env:USERPROFILE\build\aeron"
   Remove-Item -Recurse -Force "$env:USERPROFILE\build\aeron-build"
   ```
3. **Remove Registry Tweaks** (if desired)

   ```powershell
   reg delete "HKLM\SYSTEM\CurrentControlSet\Services\Afd\Parameters" /v DefaultSendWindow /f
   reg delete "HKLM\SYSTEM\CurrentControlSet\Services\Afd\Parameters" /v DefaultReceiveWindow /f
   reg delete "HKLM\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters" /v MaximumReassemblyHeaders /f
   ```

---

## 3. Troubleshooting

* **Installation Failures**: Inspect `C:\Temp\AeronInstallLog.txt` for errors.
* **CMake Errors**: Review `C:\Temp\cmake_configure.log` and `C:\Temp\cmake_build.log` for detailed output.
* **Service Errors**: Use `nssm.exe dump AeronMediaDriver` and Windows Event Viewer for NSSM logs.
* **IPC Permissions**: Aeron uses shared memory (`C:\Temp\aeron`). Ensure the service user can read/write this folder.

---

## 4. Additional Resources

* [Aeron Official Repository](https://github.com/real-logic/aeron)
* [steady\_state Framework Documentation](https://docs.rs/steady_state/)

