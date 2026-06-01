#!/usr/bin/env bash
# Install the Aeron Media Driver as a systemd service

set -euo pipefail

SERVICE_NAME="aeronmd"
SERVICE_USER=${1:-$(whoami)} # Takes the first argument or defaults to the current user
SERVICE_FILE="/etc/systemd/system/${SERVICE_NAME}.service"
DOCKER_OS="alpine" # alpine is smaller and recommended, but debian is also available
DOCKER_FILE="Dockerfile.aeronmd_${DOCKER_OS}"
DOCKER_IMAGE="aeron-${DOCKER_OS}"
DOCKER_actor="/build/binaries/aeronmd"
DESCRIPTION="steady_state:${SERVICE_NAME}"
AFTER="docker.service"
WANTED_BY="multi-user.target"
RESTART_POLICY="always"

# NOTE: you may need to run update_sysctl.sh for maximum performance and to support large buffers

# groups #SERVICE_USER
# sudo usermod -aG docker $SERVICE_USER

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
docker build -t "$DOCKER_IMAGE" -f $DOCKER_FILE .

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

# Create service group if it doesn't exist
if ! getent group "$SERVICE_USER" >/dev/null; then
    groupadd --system "$SERVICE_USER"
fi

# Create service user if it doesn't exist
if ! id -u "$SERVICE_USER" &>/dev/null; then
    useradd -r -s /usr/sbin/nologin -g "$SERVICE_USER" "$SERVICE_USER"
    # service user must be allowed to run docker
    sudo usermod -aG docker "$SERVICE_USER"

    echo "Created service user '$SERVICE_USER'."
else
    echo "Service user '$SERVICE_USER' already exists."
fi

# Get UID and GID of the user
USER_ID=$(id -u $SERVICE_USER 2>/dev/null)
GROUP_ID=$(id -g $SERVICE_USER 2>/dev/null)

loginctl enable-linger $SERVICE_USER

cat > "$SERVICE_FILE" <<EOF
[Unit]
Description=$DESCRIPTION
After=$AFTER

[Service]
ExecStart=/usr/bin/docker run --rm --cpuset-cpus="0-2" --ipc=host --network=host \
 --shm-size=4g --cap-add=sys_nice --cap-add=ipc_lock --cap-add=SYS_ADMIN \
 -v /dev/shm:/dev/shm -u ${USER_ID}:${GROUP_ID} --name $SERVICE_NAME $DOCKER_IMAGE \
 nice -10 $DOCKER_actor
User=$SERVICE_USER
Group=$SERVICE_USER
UMask=0002
Nice=-10
Restart=$RESTART_POLICY
ExecStop=/usr/bin/docker stop $SERVICE_NAME
LimitRTPRIO=99
LimitMEMLOCK=infinity
AmbientCapabilities=CAP_SYS_NICE

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
echo "Use 'journalctl -u $SERVICE_NAME -n 50 -f' to tail its logs."
sleep 5
systemctl --no-pager status $SERVICE_NAME


#The C Media Driver supports a subset of configuration options which are relevant for the Driver, i.e.:
# aeron.event.log.filename (or as AERON_EVENT_LOG_FILENAME env variable): System property for the file to which the log is appended. If not set then STDOUT will be used. Logging to file is significantly more efficient than logging to STDOUT.
# aeron.event.log (or as AERON_EVENT_LOG env variable): System property containing a comma separated list of Driver event codes. These are also special events: all for all possible events and admin for administration events.
# aeron.event.log.disable (or as AERON_EVENT_LOG_DISABLE env variable): System property containing a comma separated list of Driver events to exclude.

# More details at:  https://github.com/real-logic/aeron/wiki

# Easy test to confirm aeronmd is working, run in two terminals
# docker exec -it aeronmd /build/binaries/basic_publisher -s 123
# docker exec -it aeronmd /build/binaries/basic_subscriber -s 123
# or
# docker exec -it aeronmd /build/binaries/streaming_publisher -s=123 -m 1000 -L 8