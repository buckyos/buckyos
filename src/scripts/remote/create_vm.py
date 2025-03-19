#!/usr/bin/env python3

import sys
import os
import json
import subprocess
import time
from remote_device import remote_device

def print_usage():
    print("Usage: create_vm.py config_file")
    print("Uses the same config file format as remote_device")
    sys.exit(1)

class VMCreator:
    def __init__(self, config_path: str):
        with open(config_path, 'r') as f:
            self.devices = json.load(f)
        
        # 验证配置文件
        self._validate_config()
        
    def _validate_config(self):
        """验证配置文件格式"""
        for device_id, device_config in self.devices.items():
            required_fields = ['node_id', 'vm']
            if not all(field in device_config for field in required_fields):
                raise ValueError(f"Device {device_id} must contain: {required_fields}")
            
            if 'vm' in device_config:
                vm_config = device_config['vm']
                if 'network' in vm_config and 'type' not in vm_config['network']:
                    raise ValueError(f"VM network config for {device_id} must specify type (bridge/nat)")
    
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

    def _create_vm(self, device_id: str, device_config: dict):
        """创建单个虚拟机"""
        print(f"\nCreating VM: {device_id}")
        
        vm_config = device_config['vm']
        
        # 基本参数
        cpu = vm_config.get('cpu', 1)
        memory = vm_config.get('memory', '1G')
        disk = vm_config.get('disk', '10G')
        
        # 创建VM的基本命令
        cmd = f"multipass launch --name {device_id} --cpus {cpu} --memory {memory} --disk {disk} --cloud-init vm_init.yaml "
        
        # 添加网络配置
        if 'network' in vm_config:
            net_config = vm_config['network']
            if net_config['type'] == 'bridge':
                cmd += f"--network name={net_config['bridge']} "
        
        # 启动VM
        self._run_command(cmd)
        time.sleep(5)  # 等待VM完全启动
    
        
        # 配置hostname
        self._run_command(f"multipass exec {device_id} -- sudo hostnamectl set-hostname {device_id}")

        
        print(f"VM {device_id} created successfully")

    

    def create_all(self):
        """创建所有配置的虚拟机"""
        # 检查是否已存在同名VM
        stdout, _ = self._run_command("multipass list", check=False)
        existing_vms = [line.split()[0] for line in stdout.split('\n')[1:] if line.strip()]
        
        for device_id, device_config in self.devices.items():
            if 'vm' not in device_config:
                continue
                
            if device_id in existing_vms:
                print(f"Warning: VM {device_id} already exists, skipping...")
                continue
            
            try:
                self._create_vm(device_id, device_config)
            except Exception as e:
                print(f"Failed to create VM {device_id}: {str(e)}")
                continue

        # TODO: 通过multipass list 获取所有vm的ip

def main():
    if len(sys.argv) != 2:
        print_usage()
    
    config_file = sys.argv[1]
    if not os.path.exists(config_file):
        print(f"Config file not found: {config_file}")
        sys.exit(1)
    
    try:
        creator = VMCreator(config_file)
        creator.create_all()
    except Exception as e:
        print(f"Error: {str(e)}")
        sys.exit(1)

if __name__ == "__main__":
    main()
