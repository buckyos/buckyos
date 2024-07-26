#!/bin/bash

service_name="nginx"

command -v $service_name &> /dev/null

# 检查服务是否已安装
if [ $? -eq 0 ]; then
    echo "$service_name 服务已安装。"

    # 检查服务是否在运行
    if systemctl is-active --quiet $service_name; then
        echo "$service_name 服务正在运行。"
        exit 0
    else
        echo "$service_name 服务未运行。"
        exit 1
    fi
else
    echo "$service_name 服务未安装。"
    exit 255
fi
