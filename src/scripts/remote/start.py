#!/usr/bin/env python3

import sys
import os
import time
from control import remote_device

def print_usage():
    print("Usage: start.py <device_id> [app_id]")
    print("  app_id: Optional. If not specified, all configured apps will be started")
    sys.exit(1)

def start_app(device: remote_device, app_id: str) -> bool:
    """启动单个应用"""
    if 'apps' not in device.config:
        raise Exception(f"No apps configured for device {device.device_id}")
    
    app_config = device.config['apps'].get(app_id)
    if not app_config:
        raise Exception(f"App {app_id} not found in configuration")
    
    # 获取应用配置
    binary = app_config.get('binary', f'/opt/buckyos/bin/{app_id}')
    working_dir = app_config.get('working_dir', '/opt/buckyos')
    config_file = app_config.get('config', f'/opt/buckyos/etc/{app_id}.conf')
    
    # 检查二进制文件是否存在
    stdout, stderr = device.run_command(f"test -f {binary} && echo 'exists'")
    if 'exists' not in stdout:
        raise Exception(f"Binary {binary} not found")
    
    # 检查是否已经在运行
    stdout, stderr = device.run_command(f"pgrep -f {binary}")
    if stdout.strip():
        print(f"Warning: {app_id} is already running")
        return True
    
    # 构建启动命令
    start_cmd = f"cd {working_dir} && "
    if app_config.get('use_nohup', True):
        start_cmd += "nohup "
    
    start_cmd += f"{binary}"
    
    if os.path.exists(config_file):
        start_cmd += f" --config {config_file}"
    
    # 添加额外的启动参数
    if 'args' in app_config:
        start_cmd += f" {app_config['args']}"
    
    if app_config.get('use_nohup', True):
        start_cmd += f" > /opt/buckyos/log/{app_id}.log 2>&1 &"
    
    # 执行启动命令
    stdout, stderr = device.run_command(start_cmd)
    if stderr:
        print(f"Warning while starting {app_id}: {stderr}")
    
    # 验证进程是否启动成功
    time.sleep(1)  # 等待进程启动
    stdout, stderr = device.run_command(f"pgrep -f {binary}")
    if not stdout.strip():
        raise Exception(f"Failed to start {app_id}")
    
    print(f"Successfully started {app_id}")
    return True

def start_all_apps(device: remote_device) -> bool:
    """启动所有配置的应用"""
    if 'apps' not in device.config:
        raise Exception(f"No apps configured for device {device.device_id}")
    
    success = True
    for app_id in device.config['apps'].keys():
        try:
            start_app(device, app_id)
        except Exception as e:
            print(f"Failed to start {app_id}: {str(e)}", file=sys.stderr)
            success = False
    
    return success

def main():
    if len(sys.argv) < 2:
        print_usage()
    
    device_id = sys.argv[1]
    app_id = sys.argv[2] if len(sys.argv) > 2 else None
    
    try:
        device = remote_device(device_id)
        
        # 确保日志目录存在
        device.run_command("mkdir -p /opt/buckyos/log")
        
        if app_id:
            success = start_app(device, app_id)
        else:
            success = start_all_apps(device)
        
        sys.exit(0 if success else 1)
        
    except Exception as e:
        print(f"Error: {str(e)}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
