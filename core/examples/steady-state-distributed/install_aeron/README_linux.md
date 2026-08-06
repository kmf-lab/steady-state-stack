# Aeron Media Driver Service Setup Guide

This guide will walk you through the steps to install, configure, and run the Aeron Media Driver (`aeronmd`) service. This is designed to work with applications built using the **steady_state** framework in Rust. The service setup is particularly tailored for users unfamiliar with Aeron or IPC-based communication and ensures correct permissions and configurations for seamless operation.

---

## Prerequisites

Before proceeding, ensure the following:

1. **Supported OS:** Your system is Linux-based (Debian or Alpine recommended).
2. **Bash Scripts:** You have the provided bash scripts:
    - `install_aeronmd.sh`
    - `uninstall_aeronmd.sh`
3. **User Requirement:** You know the username under which your **steady_state** application will run. This is crucial for IPC communication, as the Aeron Media Driver and the application must have matching permissions.
4. **Dependencies:** Make sure `bash`, `docker`, and necessary privileges to run the scripts are available on your system.

---

## Installation Steps

### 1. Pass the Desired Username

The Aeron Media Driver requires a specific user to run. The `user` parameter should be passed during installation. This ensures that the Aeron Media Driver IPC permissions align with your **steady_state** application.

### 2. Install the Aeron Media Driver

Run the following command to install the Aeron Media Driver service:

```bash
bash install_aeronmd.sh <user>
```

#### Parameters:
- `<user>`: Replace this with the username under which your **steady_state** application will run.

#### Example:
If your **steady_state** application runs as the user `steady_user`, run:

```bash
bash install_aeronmd.sh <steady_user>
```

### 3. Verify Installation

To verify that the service is installed and running correctly:

1. Check the status of the Aeron Media Driver:
   ```bash
   systemctl status aeronmd.service
   ```

2. Confirm that the service is running under the specified user:
   ```bash
   ps -u <user> | grep aeronmd
   ```

---

## Uninstalling the Aeron Media Driver

If you need to uninstall the Aeron Media Driver for any reason, use the provided `uninstall_aeronmd.sh` script. Run the following command:

```bash
bash uninstall_aeronmd.sh <steady_user>
```

This will stop the Aeron Media Driver service and clean up any related files and configurations.

---

## Configuration Notes

1. **Permissions:**
    - Aeron uses **IPC (Inter-Process Communication)** shared memory for low-latency message passing. To ensure proper operation, the Aeron Media Driver must run as the same user as the **steady_state** application.

2. **Custom Directory for Shared Memory:**
    - By default, Aeron places shared memory files in `/dev/shm`. Ensure this directory has adequate space and permissions for the specified user.

3. **Docker Support:**
    - The service can also be built and run using Docker. If using Docker, ensure the container's user matches the host user running the **steady_state** application.

---

## Troubleshooting

1. **IPC Permission Errors:**
    - Ensure the Aeron Media Driver and your application are running under the same user.

2. **Service Not Starting:**
    - Check the logs for errors:
      ```bash
      journalctl -u aeronmd.service -n 50 -f
      ```

3. **Docker Issues:**
    - Ensure the correct `Dockerfile` is used (`Dockerfile.aeronmd_debian` or `Dockerfile.aeronmd_alpine`) and the container user matches the application user.

---

## Additional Resources

For more information about Aeron and its configuration, refer to:
- [Aeron Official Documentation](https://github.com/real-logic/aeron)
- [steady_state Framework Documentation](https://docs.rs/steady_state/)

