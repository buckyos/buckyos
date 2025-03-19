#!/bin/bash
set -x

# Install and configure Samba
if ! command -v nginx &> /dev/null; then
    echo "Nginx service is not installed, starting the installation process."
    sudo apt-get install -y nginx
fi

echo "Nginx deployment is complete."
