#!/usr/bin/env python3

import sys
import time
from control import remote_device

def print_usage():
    print("Usage: stop.py <device_id> [app_id]")
    print("  app_id: Optional. If not specified, all configured apps will be stopped")
    sys.exit(1)

def stop_app(device: remote_device, app_id: str) -> bool:
    """停止单个应用"""
    if 'apps' not in device.config:
        raise Exception(f"No apps configured for device {device.device_id}")
    
    app_config = device.config['apps'].get(app_id)
    if not app_config:
        raise Exception(f"App {app_id} not found in configuration")
    
    binary = app_config.get('binary', f'/opt/buckyos/bin/{app_id}')
    
    # 获取进程ID
    stdout, stderr = device.run_command(f"pgrep -f {binary}")
    pids = stdout.strip().split('\n')
    
    if not pids or not pids[0]:
        print(f"No running process found for {app_id}")
        return True
    
    success = True
    for pid in pids:
        pid = pid.strip()
        if not pid:
            continue
            
        print(f"Stopping {app_id} (PID: {pid})...")
        
        # 首先尝试正常终止进程
        stdout, stderr = device.run_command(f"kill {pid}")
        
        # 等待进程终止
        for _ in range(5):  # 最多等待5秒
            stdout, stderr = device.run_command(f"kill -0 {pid} 2>/dev/null || echo 'stopped'")
            if 'stopped' in stdout:
                break
            time.sleep(1)
        else:
            # 如果进程仍然存在，使用SIGKILL强制终止
            print(f"Force stopping {app_id} (PID: {pid})...")
            stdout, stderr = device.run_command(f"kill -9 {pid}")
            
            # 最后检查一次
            stdout, stderr = device.run_command(f"kill -0 {pid} 2>/dev/null || echo 'stopped'")
            if 'stopped' not in stdout:
                print(f"Failed to stop {app_id} (PID: {pid})", file=sys.stderr)
                success = False
                continue
        
        print(f"Successfully stopped {app_id} (PID: {pid})")
    
    return success

def stop_all_apps(device: remote_device) -> bool:
    """停止所有配置的应用"""
    if 'apps' not in device.config:
        raise Exception(f"No apps configured for device {device.device_id}")
    
    # 按照配置文件中的顺序反向停止服务
    app_ids = list(device.config['apps'].keys())
    app_ids.reverse()
    
    success = True
    for app_id in app_ids:
        try:
            if not stop_app(device, app_id):
                success = False
        except Exception as e:
            print(f"Failed to stop {app_id}: {str(e)}", file=sys.stderr)
            success = False
    
    return success

def main():
    if len(sys.argv) < 2:
        print_usage()
    
    device_id = sys.argv[1]
    app_id = sys.argv[2] if len(sys.argv) > 2 else None
    
    try:
        device = remote_device(device_id)
        
        if app_id:
            success = stop_app(device, app_id)
        else:
            success = stop_all_apps(device)
        
        sys.exit(0 if success else 1)
        
    except Exception as e:
        print(f"Error: {str(e)}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
