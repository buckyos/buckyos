#!/bin/bash
set -x

# Check if at least one argument(samba_dir) was provided
if [ "$#" -lt 1 ]; then
    echo "Usage: $0 <samba_dir>"
    exit 1
fi

echo "The provided samba_dir is: $1"

# Read the command line argument
mount_dir=$1

samba_conf="/etc/samba/smb.conf"

# Check and modify mount directory permissions
#sudo chown "$current_user":"$current_group" "$mount_dir"
#sudo chmod 770 "$mount_dir"
# 这里对匿名用户开放读写，可能会有安全问题  TODO
sudo chown nobody:nogroup "$mount_dir"
sudo chmod 777 "$mount_dir"

# Install and configure Samba
if ! command -v smbd &> /dev/null; then
    echo "Samba service is not installed, starting the installation process."
    sudo apt-get install -y samba
fi

# Configure Samba for anonymous access
sudo sed -i '/^\[global\]/a \
   map to guest = Bad User \
   guest account = nobody' "$samba_conf"

# Configure Samba share for anonymous access
sudo sed -i '/\[GlusterFS\]/,/^\[/ s/^.*$//' "$samba_conf"  # Remove the existing GlusterFS block
sudo tee -a "$samba_conf" <<EOT

[GlusterFS]
    path = $mount_dir
    browsable = yes
    writable = yes
    guest ok = yes
    guest only = yes
    create mask = 0660
    directory mask = 0770
    force user = nobody
    force group = nogroup

EOT

sudo systemctl restart smbd

echo "GlusterFS and Samba deployment for anonymous access is complete."
