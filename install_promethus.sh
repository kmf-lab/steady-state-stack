#!/bin/bash

# Exit immediately if a command exits with a non-zero status
set -e

# Create a Prometheus user with no home directory and no login shell
echo "Creating Prometheus user..."
sudo useradd --no-create-home --shell /bin/false prometheus || true

# Download the Prometheus tarball
echo "Downloading Prometheus..."
wget https://github.com/prometheus/prometheus/releases/download/v2.54.1/prometheus-2.54.1.linux-amd64.tar.gz

# Extract the downloaded tarball
echo "Extracting Prometheus..."
tar -xvf prometheus-2.54.1.linux-amd64.tar.gz

# Move the Prometheus binaries to /usr/local/bin
echo "Moving Prometheus binaries to /usr/local/bin..."
sudo mv prometheus-2.54.1.linux-amd64/prometheus /usr/local/bin/
sudo mv prometheus-2.54.1.linux-amd64/promtool /usr/local/bin/

# Create the Prometheus data directory
echo "Creating Prometheus data directory..."
sudo mkdir -p /var/lib/prometheus
sudo chown -R prometheus:prometheus /var/lib/prometheus
sudo chmod -R 755 /var/lib/prometheus

# Set ownership of Prometheus binaries
echo "Setting ownership of Prometheus binaries..."
sudo chown -R prometheus:prometheus /usr/local/bin/prometheus
sudo mkdir -p /etc/prometheus
sudo chown -R prometheus:prometheus /etc/prometheus

# Create the Prometheus systemd service file
echo "Creating Prometheus systemd service file..."
echo "[Unit]
Description=Prometheus Monitoring
After=network.target

[Service]
User=prometheus
Group=prometheus
ExecStart=/usr/local/bin/prometheus \
  --config.file=/etc/prometheus/prometheus.yml \
  --storage.tsdb.path=/var/lib/prometheus/ \
  --web.console.templates=/usr/local/share/prometheus/consoles \
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

# Create the Prometheus configuration file
echo "Creating Prometheus configuration file..."
echo "global:
  scrape_interval: 5s

scrape_configs:
  - job_name: 'prometheus'
    static_configs:
      - targets: ['localhost:9100']
" | sudo tee /etc/prometheus/prometheus.yml

# Instructions for accessing Prometheus
echo "Installation complete. You can access Prometheus in your browser at http://<your-server-ip>:9090"

echo "Check the status of the Prometheus service:"
echo "sudo systemctl status prometheus"

