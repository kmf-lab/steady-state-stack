#!/usr/bin/env bash
# Installs NanoMQ and sets it up as a systemd service on Ubuntu

set -euo pipefail

NANOMQ_VERSION="0.21.7" # Update as needed
NANOMQ_URL="https://github.com/nanomq/nanomq/releases/download/v${NANOMQ_VERSION}/nanomq-v${NANOMQ_VERSION}-Linux-x86_64.tar.gz"
INSTALL_DIR="/opt/nanomq"
SERVICE_FILE="/etc/systemd/system/nanomq.service"

echo "Installing dependencies..."
sudo apt-get update
sudo apt-get install -y curl tar

echo "Downloading NanoMQ $NANOMQ_VERSION..."
sudo mkdir -p "$INSTALL_DIR"
curl -L "$NANOMQ_URL" | sudo tar xz -C "$INSTALL_DIR" --strip-components=1

echo "Creating systemd service file..."
sudo tee "$SERVICE_FILE" > /dev/null <<EOF
[Unit]
Description=NanoMQ MQTT Broker
After=network.target

[Service]
Type=simple
ExecStart=$INSTALL_DIR/nanomq
WorkingDirectory=$INSTALL_DIR
Restart=always
User=nobody
Group=nogroup

[Install]
WantedBy=multi-user.target
EOF

echo "Reloading systemd and enabling NanoMQ service..."
sudo systemctl daemon-reload
sudo systemctl enable nanomq
sudo systemctl start nanomq

echo "NanoMQ installed and running as a systemd service."
echo "Check status: sudo systemctl status nanomq"
echo "View logs:    sudo journalctl -u nanomq -f"