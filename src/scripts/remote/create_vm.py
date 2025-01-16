#!/usr/bin/env python3

import sys
import os
import json
import subprocess
import time
from control import remote_device

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
            required_fields = ['hostname', 'username', 'vm']
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
        cmd = f"multipass launch --name {device_id} --cpus {cpu} --memory {memory} --disk {disk} --cloud-init user_config.yaml "
        
        # 添加网络配置
        if 'network' in vm_config:
            net_config = vm_config['network']
            if net_config['type'] == 'bridge':
                cmd += f"--network name={net_config['bridge']} "
        
        # 启动VM
        self._run_command(cmd)
        time.sleep(5)  # 等待VM完全启动
        
        # 配置静态IP（如果指定）
        if 'ip' in vm_config:
            self._configure_static_ip(device_id, vm_config)
        
        # 配置hostname
        self._run_command(f"multipass exec {device_id} -- sudo hostnamectl set-hostname {device_id}")
        
        # 配置hosts文件
        self._configure_hosts(device_id)
        
        # 配置SSH密钥
        if 'ssh_key' in device_config:
            self._configure_ssh(device_id, device_config)
        
        print(f"VM {device_id} created successfully")

    def _configure_static_ip(self, device_id: str, vm_config: dict):
        """配置静态IP"""
        net_config = vm_config['network']
        netplan_config = {
            "network": {
                "version": 2,
                "ethernets": {
                    "eth0": {
                        "addresses": [f"{vm_config['ip']}/24"],
                        "gateway4": net_config.get('gateway'),
                        "nameservers": {
                            "addresses": ["8.8.8.8", "8.8.4.4"]
                        }
                    }
                }
            }
        }
        
        # 写入netplan配置
        config_str = json.dumps(netplan_config)
        self._run_command(f"""multipass exec {device_id} -- bash -c 'echo \'{config_str}\' | sudo tee /etc/netplan/50-cloud-init.yaml'""")
        self._run_command(f"multipass exec {device_id} -- sudo netplan apply")
        
    def _configure_hosts(self, device_id: str):
        """配置hosts文件"""
        hosts_entries = []
        for dev_id, dev_conf in self.devices.items():
            if 'vm' in dev_conf and 'ip' in dev_conf['vm']:
                hosts_entries.append(f"{dev_conf['vm']['ip']} {dev_id}")
        
        if hosts_entries:
            hosts_str = "\n".join(hosts_entries)
            self._run_command(f"""multipass exec {device_id} -- bash -c 'echo "{hosts_str}" | sudo tee -a /etc/hosts'""")

    def _configure_ssh(self, device_id: str, device_config: dict):
        """配置SSH密钥"""
        ssh_key = device_config['ssh_key']
        username = device_config['username']
        
        # 确保.ssh目录存在
        self._run_command(f"multipass exec {device_id} -- sudo mkdir -p /home/{username}/.ssh")
        
        # 写入SSH密钥
        self._run_command(f"""multipass exec {device_id} -- bash -c 'echo "{ssh_key}" | sudo tee /home/{username}/.ssh/authorized_keys'""")
        
        # 设置正确的权限
        self._run_command(f"multipass exec {device_id} -- sudo chown -R {username}:{username} /home/{username}/.ssh")
        self._run_command(f"multipass exec {device_id} -- sudo chmod 700 /home/{username}/.ssh")
        self._run_command(f"multipass exec {device_id} -- sudo chmod 600 /home/{username}/.ssh/authorized_keys")

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
