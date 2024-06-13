#!/bin/bash

# Get the command line arguments for zone_id, server_url, and etcd_servers
server_url="http://47.106.164.184"

# Get the current directory of the shell script
install_dir="$( cd "$( dirname "${BASH_SOURCE[0]}" )" >/dev/null 2>&1 && pwd )"

# Make the file backup_service executable
chmod +x "$install_dir/backup_service"

# Print a success message
echo "backup_service file deployed successfully!"

# Create a systemd service unit file for backup_service
# Configure systemd to restart backup_service on failure
cat <<EOF > /etc/systemd/system/backup_service.service
[Unit]
Description=Backup Server
After=network.target

[Service]
ExecStart=$install_dir/backup_service --server_url="$server_url"
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF

# Reload systemd daemon to read the new unit file
systemctl daemon-reload

# Enable the backup_service service to start on boot
systemctl enable backup_service.service

# Start the backup_service service
systemctl start backup_service.service

# Print a success message
echo "Backup_service service configured and started successfully!"

