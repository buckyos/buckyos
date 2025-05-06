#!/bin/bash

cat << EOF | sudo tee /etc/sysctl.d/k8s.conf
net.ipv4.ip_forward = 1
EOF

sudo apt update
# 安装必要的依赖
echo "安装必要的依赖..."
sudo apt-get install -y curl

# 检查 docker 是否安装
if ! command -v docker &> /dev/null
then
    echo "Docker 未安装，正在安装 Docker..."
    curl -fsSL https://get.docker.com -o get-docker.sh
    sudo sh get-docker.sh
else
    echo "Docker 已安装"
fi

# 确定系统架构
ARCH=$(uname -m)
case $ARCH in
    x86_64)
        ARCH="amd64"
        ;;
    aarch64)
        ARCH="arm64"
        ;;
    *)
        echo "不支持的架构: $ARCH"
        exit 1
        ;;
esac

# 下载 cri-dockerd
echo "下载 cri-dockerd..."
CRI_DOCKERD_VERSION="0.3.12"  # 根据需要替换为最新版本
CRI_DOCKERD_URL="https://github.com/Mirantis/cri-dockerd/releases/download/v${CRI_DOCKERD_VERSION}/cri-dockerd-${CRI_DOCKERD_VERSION}.${ARCH}.tgz"
echo $CRI_DOCKERD_URL
curl -L ${CRI_DOCKERD_URL} | sudo tar -xz -C ~/ | mv -f ~/cri-dockerd/cri-dockerd /usr/local/bin

# 创建 cri-dockerd systemd unit 文件
echo "创建 cri-dockerd systemd unit 文件..."
cat <<EOF | sudo tee /etc/systemd/system/cri-docker.service
[Unit]
Description=CRI Docker Daemon
After=network.target docker.service
Requires=docker.service

[Service]
ExecStart=/usr/local/bin/cri-dockerd

[Install]
WantedBy=multi-user.target
EOF

# install k8s
sudo apt-get install -y apt-transport-https ca-certificates gpg
if [ ! -f "/etc/apt/keyrings/kubernetes-apt-keyring.gpg" ]; then
	curl -fsSL https://pkgs.k8s.io/core:/stable:/v1.30/deb/Release.key | sudo gpg --dearmor -o /etc/apt/keyrings/kubernetes-apt-keyring.gpg
fi
echo 'deb [signed-by=/etc/apt/keyrings/kubernetes-apt-keyring.gpg] https://pkgs.k8s.io/core:/stable:/v1.30/deb/ /' | sudo tee /etc/apt/sources.list.d/kubernetes.list
sudo apt-get update
sudo apt-get install -y kubelet kubeadm kubectl
sudo apt-mark hold kubelet kubeadm kubectl
