#!/usr/bin/env python3

import sys
import os
from control import remote_device

def print_usage():
    print("Usage: uninstall.py <device_id> [-clean]")
    print("Options:")
    print("  -clean    Remove entire /opt/buckyos/ directory")
    print("  without -clean: Preserve /opt/buckyos/etc and /opt/buckyos/data directories")
    sys.exit(1)

def uninstall(device_id: str, clean: bool):
    device = remote_device(device_id)
    
    if clean:
        # 完全删除整个目录
        cmd = "rm -rf /opt/buckyos/"
    else:
        # 保留etc和data目录，删除其他
        cmd = """
        cd /opt/buckyos/ && \
        find . -maxdepth 1 ! -name 'etc' ! -name 'data' ! -name '.' -exec rm -rf {} +
        """
    
    stdout, stderr = device.run_command(cmd)
    
    if stderr:
        print(f"Error during uninstall: {stderr}", file=sys.stderr)
        return False
    
    print(f"Successfully uninstalled BuckyOS from {device_id}")
    if not clean:
        print("Note: /opt/buckyos/etc and /opt/buckyos/data directories were preserved")
    return True

def main():
    # 检查参数
    if len(sys.argv) < 2:
        print_usage()
    
    device_id = sys.argv[1]
    clean_mode = "-clean" in sys.argv
    
    try:
        success = uninstall(device_id, clean_mode)
        sys.exit(0 if success else 1)
    except Exception as e:
        print(f"Error: {str(e)}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
