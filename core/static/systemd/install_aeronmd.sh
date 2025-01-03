#!/usr/bin/env bash

set -euo pipefail

SERVICE_NAME="aeronmd"
SERVICE_USER="aeronmd"
SERVICE_FILE="/etc/systemd/system/${SERVICE_NAME}.service"
DOCKER_IMAGE="aeron-alpine"
DOCKER_CMD="/build/binaries/aeronmd"
DESCRIPTION="steady_state:${SERVICE_NAME}"
AFTER="docker.service"
WANTED_BY="multi-user.target"
RESTART_POLICY="always"
NETWORK_MODE="host"

# Ensure systemd and required commands are available
if ! command -v systemctl &> /dev/null; then
    echo "systemctl not found. systemd might not be installed."
    exit 1
fi

if ! command -v docker &> /dev/null; then
    echo "docker not found. Please install Docker to use this script."
    exit 1
fi

# optional: should already be built earlier.
docker build -t "$DOCKER_IMAGE" -f ../../Dockerfile.aeronmd_alpine .

# Ensure script is run as root
if [[ $EUID -ne 0 ]]; then
    echo "Please run this script as root (e.g., using sudo)."
    exit 1
fi

# Pull the Docker image if it’s not already available
if ! docker image inspect "$DOCKER_IMAGE" &>/dev/null; then
    echo "Pulling Docker image '$DOCKER_IMAGE'..."
    docker pull "$DOCKER_IMAGE"
fi

echo "Installing $SERVICE_NAME service..."

# Create service user if it doesn't exist
if ! id -u "$SERVICE_USER" &>/dev/null; then
    useradd -r -s /usr/sbin/nologin "$SERVICE_USER"
    # service user must be allowed to run docker
    sudo usermod -aG docker "$SERVICE_USER"
    echo "Created service user '$SERVICE_USER'."
else
    echo "Service user '$SERVICE_USER' already exists."
fi

# Create the systemd service file
cat > "$SERVICE_FILE" <<EOF
[Unit]
Description=$DESCRIPTION
After=$AFTER

[Service]
ExecStart=/usr/bin/docker run --rm --network=$NETWORK_MODE --name $SERVICE_NAME $DOCKER_IMAGE $DOCKER_CMD
User=$SERVICE_USER
Restart=$RESTART_POLICY
ExecStop=/usr/bin/docker stop $SERVICE_NAME

[Install]
WantedBy=$WANTED_BY
EOF

echo "Service file created at $SERVICE_FILE."

# Reload systemd daemon
systemctl daemon-reload

# Enable service on boot
systemctl enable "$SERVICE_NAME"

# Start the service immediately
systemctl start "$SERVICE_NAME"

echo "$SERVICE_NAME service installed and started successfully."
echo "Use 'systemctl status $SERVICE_NAME' to check its status."
