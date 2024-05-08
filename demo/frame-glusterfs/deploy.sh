#!/bin/bash

# 安装GlusterFS
install_glusterfs() {
    # 等待dpkg锁释放
    # TODO 这里需要一个合适的方法，来等待dpkg锁释放
    # while sudo fuser /var/lib/dpkg/lock >/dev/null 2>&1 || sudo fuser /var/lib/dpkg/lock-frontend >/dev/null 2>&1; do
    #   echo "Waiting for other dpkg operations to complete..."
    #   sudo systemctl stop unattended-upgrades
    #   sleep 10
    # done

    if ! which glusterd >/dev/null 2>&1; then
        echo "Installing GlusterFS..."
        sudo apt-get update && sudo apt-get install -y glusterfs-server
        if [ $? -ne 0 ]; then
            echo "Failed to install GlusterFS. Exiting."
            exit 1
        fi
    else
        echo "GlusterFS already installed."
    fi
}

# 主函数
main() {
    install_glusterfs
}

main
