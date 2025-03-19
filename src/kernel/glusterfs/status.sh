#!/bin/bash
# Check if 4 arguments was provided
if [ "$#" -ne 2 ]; then
    echo "Usage: $0 GLUSTER_VOLUME MOUNT_POINT"
    exit 1
fi

# GlusterFS 卷名
GLUSTER_VOLUME=$1
# 挂载点
MOUNT_POINT=$2
# 安装脚本名称
DEPLOY_SCRIPT="deploy.sh --gluster"

# 检查 GlusterFS 服务是否已安装
check_glusterfs_installed() {
    if command -v gluster > /dev/null; then
        return 0
    else
        return 1
    fi
}

# 检查 GlusterFS 服务状态
check_glusterfs_service() {
    if sudo systemctl is-active --quiet glusterd; then
        return 0
    else
        return 1
    fi
}

# 检查 GlusterFS 卷的状态
check_volume_status() {
    local status
    status=$(sudo gluster volume info "$GLUSTER_VOLUME" | grep 'Status:' | awk '{print $2}')
    if [ "$status" = "Started" ]; then
        return 0
    else
        return 1
    fi
}

# 检查 GlusterFS 卷是否已挂载
check_mount_status() {
    if mount | grep -q " $MOUNT_POINT "; then
        return 0
    else
        return 1
    fi
}

# 检查 GlusterFS 安装脚本是否正在运行
check_deploy_script_running() {
    if pgrep -f "$DEPLOY_SCRIPT" > /dev/null; then
        return 0
    else
        return 1
    fi
}

# 确定 GlusterFS 状态并返回相应的状态码
determine_glusterfs_status() {
    if check_deploy_script_running; then
        echo "GlusterFS status: Deploying"
        exit 255  # Deploying 状态
    elif check_glusterfs_installed; then
        if check_glusterfs_service && check_volume_status && check_mount_status; then
            echo "GlusterFS status: Running"
            exit 0  # Running 状态
        else
            echo "GlusterFS status: Stopped"
            exit 1  # Stopped 状态
        fi
    else
        echo "GlusterFS status: Unknown"
        exit 254  # 未知状态
    fi
}

# 主函数
main() {
    determine_glusterfs_status
}

main