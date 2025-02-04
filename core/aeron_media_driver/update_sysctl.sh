#!/bin/bash

# sudo chown -R nate:nate /dev/hugepages

#sudo groupadd hugepages
#sudo usermod -aG hugepages nate
#getent group hugepages
#echo "vm.hugetlb_shm_group=????" | sudo tee -a /etc/sysctl.conf
#sudo sysctl -p
#sudo sysctl -p

#sudo chown -R :hugepages /dev/hugepages
#sudo chmod 770 /dev/hugepages

# Desired sysctl parameter values and their target values
declare -A sysctl_values=(
  ["net.core.rmem_max"]=4194304        # Max receive buffer size for all sockets (4MB)
  ["net.core.wmem_max"]=4194304        # Max send buffer size for all sockets (4MB)
  ["fs.file-max"]=4194304              # Maximum number of open file descriptors
  ["net.ipv4.udp_rmem_min"]=4194304     # Minimum UDP receive buffer per socket (4MB)
  ["net.ipv4.udp_wmem_min"]=4194304     # Minimum UDP send buffer per socket (4MB)
  ["net.ipv4.udp_mem"]="8388608 12582912 16777216"  # System-wide UDP buffer (min, pressure, max in bytes)
)

# Function to check and update sysctl values
update_sysctl() {
  local key="$1"               # The sysctl parameter name (e.g., net.core.rmem_max)
  local desired_value="$2"      # The desired value to set for this parameter
  local current_value

  # Retrieve the current value of the sysctl parameter
  # `sysctl -n <key>` prints the current value, `2>/dev/null` suppresses errors if the key doesn't exist
  current_value=$(sysctl -n "$key" 2>/dev/null)

  # If the sysctl parameter is not found, add it to /etc/sysctl.conf and set it
  if [[ $? -ne 0 ]]; then
    echo "Parameter $key not found, adding it to sysctl.conf"
    sudo sed -i "/^$key=/d" /etc/sysctl.conf  # Safe deletion: remove exact match with '='
    echo "$key=$desired_value" | sudo tee -a /etc/sysctl.conf > /dev/null
    return
  fi

  # Special case handling for space-separated values (multi-value settings)
  if [[ "$key" == "net.ipv4.udp_mem" ]]; then
    # Since net.ipv4.udp_mem contains three space-separated values, compare the entire string
    if [[ "$current_value" != "$desired_value" ]]; then
      echo "Updating $key from '$current_value' to '$desired_value'"
      sudo sed -i "/^$key=/d" /etc/sysctl.conf  # Safe deletion
      echo "$key=$desired_value" | sudo tee -a /etc/sysctl.conf > /dev/null
    else
      echo "$key is already set correctly."
    fi
  else
    # For numeric values, compare them using integer comparison (-lt = less than)
    if [[ -z "$current_value" || "$current_value" -lt "$desired_value" ]]; then
      echo "Updating $key from '$current_value' to '$desired_value'"
      sudo sed -i "/^$key=/d" /etc/sysctl.conf  # Safe deletion
      echo "$key=$desired_value" | sudo tee -a /etc/sysctl.conf > /dev/null
    else
      echo "$key is already set correctly."
    fi
  fi
}

# Process each sysctl setting by iterating over the dictionary (key-value pairs)
for key in "${!sysctl_values[@]}"; do
  update_sysctl "$key" "${sysctl_values[$key]}"
done

# Apply the changes to the system immediately using sysctl -p
echo "Applying sysctl changes..."
sudo sysctl -p

# Inform the user that settings were successfully applied
echo "Sysctl settings have been applied successfully."
