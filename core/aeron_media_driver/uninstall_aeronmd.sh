#!/usr/bin/env bash
# Uninstall the Aeron Media Driver as a systemd service

set -euo pipefail

SERVICE_NAME="aeronmd"
SERVICE_USER=${1:-$(whoami)} # Takes the first argument or defaults to the current user
SERVICE_FILE="/etc/systemd/system/${SERVICE_NAME}.service"
DOCKER_CONTAINER_NAME="${SERVICE_NAME}" # Matches the container name in the service file

# Ensure script is run as root
if [[ $EUID -ne 0 ]]; then
    echo "Please run this script as root (e.g., using sudo)."
    exit 1
fi

echo "Uninstalling $SERVICE_NAME service..."

# Stop the service if running
if systemctl is-active --quiet "$SERVICE_NAME"; then
    systemctl stop "$SERVICE_NAME"
    echo "Stopped $SERVICE_NAME service."
fi

# Disable the service from starting on boot
if systemctl is-enabled --quiet "$SERVICE_NAME"; then
    systemctl disable "$SERVICE_NAME"
    echo "Disabled $SERVICE_NAME service from starting on boot."
fi

# Remove the systemd service file
if [[ -f "$SERVICE_FILE" ]]; then
    rm -f "$SERVICE_FILE"
    echo "Removed the service file $SERVICE_FILE."
fi

# Reload systemd daemon
systemctl daemon-reload

# Stop and remove the Docker container (if it's still running for some reason)
if docker ps -q --filter "name=${DOCKER_CONTAINER_NAME}" | grep -q .; then
    docker stop "${DOCKER_CONTAINER_NAME}"
    echo "Stopped Docker container $DOCKER_CONTAINER_NAME."
fi

# Remove IPC directory
rm -fr /dev/shm/aeron-root
rm -fr /dev/shm/aeron-default

echo "$SERVICE_NAME service uninstalled successfully."
