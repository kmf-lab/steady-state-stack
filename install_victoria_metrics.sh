#!/bin/bash


# VictoriaMetrics installation script
# TODO: not yet working, needs more attention


# Exit immediately if a command exits with a non-zero status
set -e

# Download the VictoriaMetrics tarball
echo "Downloading VictoriaMetrics..."
wget https://github.com/VictoriaMetrics/VictoriaMetrics/releases/download/v1.79.2/victoria-metrics-linux-amd64-v1.79.2.tar.gz

# Extract the downloaded tarball
echo "Extracting VictoriaMetrics..."
tar -xvf victoria-metrics-linux-amd64-v1.79.2.tar.gz

# Move the VictoriaMetrics binary to /usr/local/bin
echo "Moving VictoriaMetrics binary to /usr/local/bin..."
sudo mv victoria-metrics-prod /usr/local/bin/victoria-metrics

# Create the VictoriaMetrics data directory
echo "Creating VictoriaMetrics data directory..."
sudo mkdir -p /etc/victoria-metrics-data

# Create a VictoriaMetrics user with no home directory and no login shell
echo "Creating VictoriaMetrics user..."
sudo useradd --no-create-home --shell /bin/false victoria

# Set ownership and permissions for VictoriaMetrics data directory and binary
echo "Setting ownership and permissions for VictoriaMetrics..."
sudo chown -R victoria:victoria /etc/victoria-metrics-data
sudo chown -R victoria:victoria /usr/local/bin/victoria-metrics
sudo chmod -R 755 /etc/victoria-metrics-data

# Create the VictoriaMetrics systemd service file
echo "Creating VictoriaMetrics systemd service file..."
echo "[Unit]
Description=VictoriaMetrics Time Series Database
After=network.target

[Service]
ExecStart=/usr/local/bin/victoria-metrics -retentionPeriod=1 -storageDataPath=/etc/victoria-metrics-data

Restart=always
User=victoria
Group=victoria

[Install]
WantedBy=multi-user.target" | sudo tee /etc/systemd/system/victoriametrics.service

echo "[Unit]
Description=VictoriaMetrics Agent
After=network.target

[Service]
ExecStart=/usr/local/bin/vmagent -remoteWrite.url=http://localhost:8428/api/v1/write -scrape.config=/etc/vmagent.yml
Restart=always
User=victoria
Group=victoria

[Install]
WantedBy=multi-user.target" | sudo tee /etc/systemd/system/vmagent.service

echo "global:
        scrape_interval: 15s

      scrape_configs:
        - job_name: 'victoriametrics'
          static_configs:
            - targets: ['localhost:9100']" | sudo tee /etc/vmagent.yml

# Reload systemd to apply the new service
echo "Reloading systemd..."
sudo systemctl daemon-reload

# Enable the VictoriaMetrics service to start on boot
echo "Enabling VictoriaMetrics service..."
sudo systemctl enable victoriametrics

# Start the VictoriaMetrics service
echo "Starting VictoriaMetrics service..."
sudo systemctl restart victoriametrics

# Check the status of the VictoriaMetrics service
sudo systemctl status victoriametrics
sudo systemctl enable vmagent
sudo systemctl start vmagent
# Instructions for accessing VictoriaMetrics
echo "Installation complete. You can access VictoriaMetrics in your browser at http://<your-server-ip>:8428"

