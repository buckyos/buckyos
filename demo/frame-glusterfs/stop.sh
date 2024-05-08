#!/bin/bash

# Check if 2 arguments was provided
if [ "$#" -ne 2 ]; then
    echo "Usage: $0 GLUSTER_VOLUME MOUNT_POINT"
    exit 1
fi

# GlusterFS 卷名
GLUSTER_VOLUME=$1
# 挂载点
MOUNT_POINT=$2

# 卸载 GlusterFS 卷
unmount_volume() {
    if mount | grep -q " $MOUNT_POINT "; then
        echo "Unmounting GlusterFS volume from $MOUNT_POINT"
        sudo umount "$MOUNT_POINT"
        echo "GlusterFS volume unmounted."
    else
        echo "GlusterFS volume is not mounted."
    fi
}

# 停止 GlusterFS 卷
stop_volume() {
    if gluster volume info "$GLUSTER_VOLUME" &>/dev/null; then
        echo "Stopping GlusterFS volume: $GLUSTER_VOLUME"
        sudo gluster volume stop "$GLUSTER_VOLUME" --mode=script
        echo "GlusterFS volume $GLUSTER_VOLUME stopped."
    else
        echo "GlusterFS volume $GLUSTER_VOLUME does not exist."
    fi
}

# 停止 GlusterFS 服务
stop_glusterfs_service() {
    echo "Stopping GlusterFS service..."
    sudo systemctl stop glusterd
    echo "GlusterFS service stopped."
}

# 主函数
main() {
    unmount_volume
    stop_volume
    stop_glusterfs_service
}

main