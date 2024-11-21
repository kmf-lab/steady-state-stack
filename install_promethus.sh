#!/bin/bash

# Exit immediately if a command exits with a non-zero status
set -e

# Create a Prometheus user with no home directory and no login shell
echo "Creating Prometheus user..."
sudo useradd --no-create-home --shell /bin/false prometheus || true

# Download the Prometheus tarball (Updated to version 3.0.0)
echo "Downloading Prometheus..."
wget https://github.com/prometheus/prometheus/releases/download/v3.0.0/prometheus-3.0.0.linux-amd64.tar.gz

# Extract the downloaded tarball
echo "Extracting Prometheus..."
tar -xvf prometheus-3.0.0.linux-amd64.tar.gz

# Move the Prometheus binaries to /usr/local/bin
echo "Moving Prometheus binaries to /usr/local/bin..."
sudo mv prometheus-3.0.0.linux-amd64/prometheus /usr/local/bin/
sudo mv prometheus-3.0.0.linux-amd64/promtool /usr/local/bin/

# Move console libraries and templates (Necessary for web UI)
echo "Moving console libraries and templates..."
# sudo mkdir -p /usr/local/share/prometheus/
# sudo mv prometheus-3.0.0.linux-amd64/consoles /usr/local/share/prometheus/
# sudo mv prometheus-3.0.0.linux-amd64/console_libraries /usr/local/share/prometheus/

# Create the Prometheus data directory
echo "Creating Prometheus data directory..."
sudo mkdir -p /var/lib/prometheus
sudo chown -R prometheus:prometheus /var/lib/prometheus
sudo chmod -R 755 /var/lib/prometheus

# Set ownership of Prometheus binaries and directories
echo "Setting ownership of Prometheus binaries and directories..."
sudo chown prometheus:prometheus /usr/local/bin/prometheus
sudo chown prometheus:prometheus /usr/local/bin/promtool
sudo chown -R prometheus:prometheus /usr/local/share/prometheus/
sudo mkdir -p /etc/prometheus
sudo chown -R prometheus:prometheus /etc/prometheus

# Create the Prometheus configuration file (Before starting the service)
echo "Creating Prometheus configuration file..."
echo "global:
  scrape_interval: 5s

scrape_configs:
  - job_name: 'prometheus'
    static_configs:
      - targets: ['localhost:9100']
" | sudo tee /etc/prometheus/prometheus.yml

sudo chown prometheus:prometheus /etc/prometheus/prometheus.yml

# Create the Prometheus systemd service file
echo "Creating Prometheus systemd service file..."
echo "[Unit]
Description=Prometheus Monitoring
After=network.target

[Service]
User=prometheus
Group=prometheus
ExecStart=/usr/local/bin/prometheus \\
  --config.file=/etc/prometheus/prometheus.yml \\
  --storage.tsdb.path=/var/lib/prometheus/ \\
  --web.console.templates=/usr/local/share/prometheus/consoles \\
  --web.console.libraries=/usr/local/share/prometheus/console_libraries
Restart=always

[Install]
WantedBy=multi-user.target" | sudo tee /etc/systemd/system/prometheus.service

# Reload systemd to apply the new service
echo "Reloading systemd..."
sudo systemctl daemon-reload

# Enable the Prometheus service to start on boot
echo "Enabling Prometheus service..."
sudo systemctl enable prometheus

# Start the Prometheus service
echo "Starting Prometheus service..."
sudo systemctl start prometheus

# Instructions for accessing Prometheus
echo "Installation complete. You can access Prometheus in your browser at http://<your-server-ip>:9090"

echo "Check the status of the Prometheus service:"
echo "sudo systemctl status prometheus"
