#!/usr/bin/env python3

import sys
import os
import json
import subprocess
import time

def print_usage():
    print("Usage: remove_vm.py <config_file> <device_id>")
    print("Warning: This operation will permanently delete the specified VM!")
    sys.exit(1)

class VMRemover:
    def __init__(self, config_path: str):
        with open(config_path, 'r') as f:
            self.devices = json.load(f)
    
    def _run_command(self, cmd: str, check=True) -> tuple:
        """执行shell命令"""
        try:
            result = subprocess.run(cmd, shell=True, check=check,
                                  stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE,
                                  text=True)
            return result.stdout, result.stderr
        except subprocess.CalledProcessError as e:
            print(f"Command failed: {cmd}")
            print(f"Error: {e.stderr}")
            if check:
                raise
            return e.stdout, e.stderr

    def get_vm_info(self, device_id: str) -> dict:
        """获取VM的信息"""
        if device_id not in self.devices:
            raise ValueError(f"Device {device_id} not found in configuration")
        
        device_config = self.devices[device_id]
        if 'vm' not in device_config:
            raise ValueError(f"Device {device_id} is not a VM")
        
        # 获取VM的运行状态
        stdout, _ = self._run_command(f"multipass info {device_id}", check=False)
        
        info = {
            'id': device_id,
            'config': device_config['vm'],
            'running': 'Running' in stdout if stdout else False
        }
        
        return info

    def confirm_removal(self, device_id: str) -> bool:
        """双重确认机制"""
        vm_info = self.get_vm_info(device_id)
        
        print("\nWARNING: You are about to delete the following VM:")
        print(f"  Device ID: {vm_info['id']}")
        print(f"  Status: {'Running' if vm_info['running'] else 'Stopped'}")
        print(f"  CPU: {vm_info['config'].get('cpu', 1)} cores")
        print(f"  Memory: {vm_info['config'].get('memory', '1G')}")
        print(f"  Disk: {vm_info['config'].get('disk', '5G')}")
        if 'ip' in vm_info['config']:
            print(f"  IP: {vm_info['config']['ip']}")
        print("\nThis operation cannot be undone!")
        
        # 第一次确认
        confirm1 = input("\nType 'yes' to confirm you want to delete this VM: ")
        if confirm1.lower() != 'yes':
            return False
        
        # 第二次确认
        confirm2 = input(f"\nType the device ID '{device_id}' to confirm deletion: ")
        if confirm2 != device_id:
            return False
        
        return True

    def remove_vm(self, device_id: str):
        """删除虚拟机"""
        try:
            # 获取VM信息并确认
            vm_info = self.get_vm_info(device_id)
            
            if not self.confirm_removal(device_id):
                print("\nVM removal cancelled.")
                return False
            
            print(f"\nRemoving VM {device_id}...")
            
            # 如果VM在运行，先停止它
            if vm_info['running']:
                print("Stopping VM...")
                self._run_command(f"multipass stop {device_id}")
                time.sleep(2)
            
            # 删除VM
            print("Deleting VM...")
            self._run_command(f"multipass delete {device_id}")
            
            # 清理VM
            print("Purging VM...")
            self._run_command("multipass purge")
            
            print(f"\nVM {device_id} has been successfully removed.")
            return True
            
        except Exception as e:
            print(f"Error removing VM: {str(e)}")
            return False

def main():
    if len(sys.argv) != 3:
        print_usage()
    
    config_file = sys.argv[1]
    device_id = sys.argv[2]
    
    if not os.path.exists(config_file):
        print(f"Config file not found: {config_file}")
        sys.exit(1)
    
    try:
        remover = VMRemover(config_file)
        success = remover.remove_vm(device_id)
        sys.exit(0 if success else 1)
    except Exception as e:
        print(f"Error: {str(e)}")
        sys.exit(1)

if __name__ == "__main__":
    main()
