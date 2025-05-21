#!/usr/bin/env python3

import sys
import os
import time
from remote_device import remote_device

def print_usage():
    print("Usage: stop.py device_id [app_id]")
    print("  app_id: Optional. If not specified, all configured apps will be started")
    sys.exit(1)

def stop_app(device: remote_device, app_id: str) -> bool:

    app_config = device.get_app_config(app_id)
    if app_config is None:
        raise Exception(f"App {app_id} not found in configuration")

    start_cmd = app_config.get('stop')
    if start_cmd is None:
        raise Exception(f"stop command for {app_id} not found in configuration")
    
    # 执行启动命令
    stdout, stderr = device.run_command(start_cmd)
    if stderr:
        print(f"Warning while stop {app_id}: {stderr}")
    
    time.sleep(1)  # 等待进程启动
    
    print(f"Successfully stop {app_id}")
    return True

def stop_all_apps(device: remote_device) -> bool:
    success = True
    for app_id in device.apps.keys():
        try:
            stop_app(device, app_id)
        except Exception as e:
            print(f"Failed to stop {app_id}: {str(e)}", file=sys.stderr)
            success = False
    
    return success

def main():
    if len(sys.argv) < 2:
        print_usage()
    
    device_id = sys.argv[1]
    device = remote_device(device_id)
    app_id = sys.argv[2] if len(sys.argv) > 2 else None
    
    try:
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
