#!/bin/bash
set -x

# Check if at least one argument(hostname) was provided
if [ "$#" -lt 1 ]; then
    echo "Usage: $0 <hostname>"
    exit 1
fi

echo "The provided hostname is: $1"

# Read the command line argument
hostname=$1

glusterfs_data_dir="/glusterfs/distributed"
mount_dir="/mnt/gv0"
samba_conf="/etc/samba/smb.conf"

if [ -n "$SUDO_USER" ]; then
    current_user="$SUDO_USER"   ## 如果使用sudo运行，whoami会是root，所以这里要获取SUDO_USER
else
    current_user="$(whoami)"
fi
current_group="$(id -gn $current_user)"  # 获取当前用户的组

# Check if GlusterFS service is installed
if ! command -v glusterd &> /dev/null; then
    echo "GlusterFS service is not installed, starting the installation process."
    sudo apt-get update -y
    sudo apt-get install -y glusterfs-server
    sudo systemctl start glusterd
    sudo systemctl enable glusterd
else
    echo "GlusterFS service is already installed."
fi

# Create GlusterFS data directory
sudo mkdir -p "$glusterfs_data_dir"

# Create GlusterFS volume
if ! sudo gluster volume info gv0 &> /dev/null; then
    echo "Creating GlusterFS volume gv0."
    sudo gluster volume create gv0 $hostname:"$glusterfs_data_dir" force
    sudo gluster volume start gv0
else
    echo "GlusterFS volume gv0 already exists."
fi

# Ensure the mount point directory exists
sudo mkdir -p "$mount_dir"

# Mount GlusterFS volume
if ! mount | grep -q " $mount_dir "; then
    echo "Mounting GlusterFS volume gv0."
    echo "$hostname:/gv0 $mount_dir glusterfs defaults,_netdev,backupvolfile-server=$hostname 0 0" | sudo tee -a /etc/fstab
    sudo mount -a
else
    echo "GlusterFS volume gv0 is already mounted to $mount_dir."
fi

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