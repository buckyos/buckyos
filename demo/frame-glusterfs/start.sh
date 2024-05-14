#!/bin/bash

# 配置
HOST_NAME=$1
GLUSTER_VOLUME=$2
BRICK_PATH=$3
MOUNT_POINT=$4
# NODES 是作为一个字符串传入，内部包含所有节点，以空格分隔
# TODO 这里更好的做法是先让节点上报到etcd，然后从etcd获取所有节点
NODES=($5)

# Check if 5 arguments was provided
if [ "$#" -ne 5 ]; then
    echo "Usage: $0 HOST_NAME GLUSTER_VOLUME BRICK_PATH MOUNT_POINT 'NODE1 NODE2 ...'"
    exit 1
fi

# 确保 brick 目录存在并设置正确的权限
ensure_brick_path() {
    sudo mkdir -p "$BRICK_PATH"
    sudo chown -R nobody:nogroup "$BRICK_PATH"
    sudo chmod -R 0755 "$BRICK_PATH"
}

# 确保 GlusterFS 服务运行
ensure_glusterfs_running() {
    echo "Ensuring GlusterFS service is running..."
    sudo systemctl enable glusterd
    sudo systemctl start glusterd
    echo "GlusterFS service is running."
}

# 添加信任节点
probe_peers() {
    local max_retries=50
    local delay=15
    for node in "${NODES[@]}"; do
        if [ "$HOST_NAME" != "$node" ]; then
            local success=0
            for ((i=0; i<max_retries; i++)); do
                echo "Probing peer $node, attempt $(($i + 1))..."
                if sudo gluster peer probe "$node"; then
                success=1
                break
                else
                echo "Probe failed, will retry in $delay seconds..."
                sleep $delay
                fi
            done
            if [ $success -ne 1 ]; then
                echo "Failed to probe peer $node after $max_retries attempts. Exiting."
                exit 1
            fi
        fi
    done
}

# 等待所有节点成为一部分
wait_for_peers() {
    local peer_count=${#NODES[@]}
    local connected_peers
    local retries=0
    while : ; do
        connected_peers=$(sudo gluster peer status | grep -c 'State: Peer in Cluster (Connected)')
        if [ "$connected_peers" -eq $((peer_count - 1)) ]; then
            break
        else
            retries=$((retries+1))
        if [ "$retries" -gt 60 ]; then  # 60 retries, approximately 5 minutes
            echo "Peers did not connect after multiple retries. Exiting."
            exit 1
        fi
            sleep 5
        fi
    done
}

# 创建分布式卷
create_volume() {
    if [ "$HOST_NAME" == "etcd1" ]; then
        wait_for_peers
        
        if ! sudo gluster volume info $GLUSTER_VOLUME >/dev/null 2>&1; then
            echo "Creating GlusterFS volume: $GLUSTER_VOLUME"
            sudo gluster volume create $GLUSTER_VOLUME transport tcp $(IFS=','; echo "${NODES[*]/%/:$BRICK_PATH}" | sed 's/,/ /g') force
            sudo gluster volume start $GLUSTER_VOLUME
            echo "GlusterFS volume $GLUSTER_VOLUME created and started."
        else
            echo "GlusterFS volume $GLUSTER_VOLUME already exists."
        fi
    else
        # 等待卷在 etcd1 上创建
        while ! gluster volume info "$GLUSTER_VOLUME" &>/dev/null; do
            echo "Waiting for volume $GLUSTER_VOLUME to be created on etcd1..."
            sleep 5
        done
    fi
}

# 确保GlusterFS卷已经启动
ensure_volume_started() {
    if ! gluster volume status $GLUSTER_VOLUME &> /dev/null; then
        echo "Starting GlusterFS $GLUSTER_VOLUME..."
        gluster volume start $GLUSTER_VOLUME
        if ! gluster volume status $GLUSTER_VOLUME &> /dev/null; then
            echo "Start GlusterFS $GLUSTER_VOLUME failed. Exiting."
            exit 1
        fi
    else
        echo "GlusterFS $VOLUME_NAME already started."
    fi
}

# 主函数
main() {
    ensure_glusterfs_running

    ensure_brick_path
    probe_peers
    create_volume

    ensure_volume_started
}

main
