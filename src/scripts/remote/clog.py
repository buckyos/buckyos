#!/usr/bin/env python3

import sys
import os
import subprocess
from control import remote_device

def print_usage():
    print("Usage: clog.py [device_id]")
    print("  device_id: Optional. If not specified, logs from all devices will be downloaded")
    sys.exit(1)

def ensure_local_dir(device_id: str) -> str:
    """确保本地日志目录存在"""
    log_dir = f"/tmp/buckyos_logs/{device_id}"
    # 如果目录已存在，先清空它
    if os.path.exists(log_dir):
        subprocess.run(f"rm -rf {log_dir}/*", shell=True, check=True)
    else:
        os.makedirs(log_dir)
    return log_dir

def get_device_log(device: remote_device) -> bool:
    """获取单个设备的日志"""
    try:
        device_id = device.device_id
        print(f"\nCollecting logs from {device_id}...")
        
        # 创建本地目录
        local_dir = ensure_local_dir(device_id)
        
        # 检查日志目录是否存在
        stdout, stderr = device.run_command("test -d /opt/buckyos/log && echo 'exists'")
        if 'exists' not in stdout:
            print(f"No log directory found on {device_id}")
            return True
        
        # 直接使用scp下载整个日志目录
        print(f"Downloading logs from {device_id}...")
        scp_command = f"scp -r -P {device.config.get('port', 22)} {device.config['username']}@{device.config['hostname']}:/opt/buckyos/log/* {local_dir}/"
        subprocess.run(scp_command, shell=True, check=True)
        
        print(f"Logs from {device_id} saved to {local_dir}")
        return True
        
    except Exception as e:
        print(f"Error collecting logs from {device_id}: {str(e)}", file=sys.stderr)
        return False

def get_all_logs() -> bool:
    """获取所有设备的日志"""
    import json
    import os
    
    # 从control.py的配置文件中读取所有设备
    config_path = os.path.expanduser('~/.remote_devices/config.json')
    try:
        with open(config_path, 'r') as f:
            configs = json.load(f)
    except FileNotFoundError:
        raise Exception("Configuration file not found")
    
    success = True
    for device_id in configs.keys():
        try:
            device = remote_device(device_id)
            if not get_device_log(device):
                success = False
        except Exception as e:
            print(f"Failed to get logs from {device_id}: {str(e)}", file=sys.stderr)
            success = False
    
    return success

def main():
    if len(sys.argv) > 2:
        print_usage()
    
    try:
        if len(sys.argv) == 2:
            device_id = sys.argv[1]
            device = remote_device(device_id)
            success = get_device_log(device)
        else:
            success = get_all_logs()
        
        sys.exit(0 if success else 1)
        
    except Exception as e:
        print(f"Error: {str(e)}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
