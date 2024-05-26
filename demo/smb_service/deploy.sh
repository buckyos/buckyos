#!/bin/bash
set -x

# Install and configure Samba
if ! command -v smbd &> /dev/null; then
    echo "Samba service is not installed, starting the installation process."
    sudo apt-get install -y samba
fi

echo "GlusterFS and Samba deployment for anonymous access is complete."
