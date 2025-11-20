import sys
import os
import json
import subprocess
import time


current_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.append(current_dir)
import util
import vm_mgr

# create vm by read demo_env.json
def create():
    if not os.path.exists(util.ENV_CONFIG):
        print(f"Config file not found: {util.ENV_CONFIG}")
        sys.exit(1)
    print(f"Using config file: {util.ENV_CONFIG}")
    creator = VMCreator(util.ENV_CONFIG, util.CONFIG_BASE)
    creator.create_all()


class VMCreator:
    def __init__(self, config_path: str, config_base: str):
        with open(config_path, 'r') as f:
            self.devices = json.load(f)
        
        # 验证配置文件
        self._validate_config()
        self.config_base = config_base
        # 初始化 VM 管理器
        self.vm_manager = vm_mgr.VMManager(backend_type="multipass")
        
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

    def _create_vm(self, device_id: str, device_config: dict):
        """创建单个虚拟机"""
        print(f"\nCreating VM: {device_id}")
        
        vm_config = device_config['vm']
        
        # 准备 VM 配置
        vm_create_config = {
            'cpu': vm_config.get('cpu', 1),
            'memory': vm_config.get('memory', '1G'),
            'disk': vm_config.get('disk', '10G'),
            'config_base': self.config_base
        }
        
        # 使用 vm_mgr 创建 VM
        print(f"Creating VM with config: {vm_create_config}")
        success = self.vm_manager.create_vm(device_id, vm_create_config)
        
        if not success:
            raise Exception(f"Failed to create VM {device_id}")
        
        time.sleep(5)  # 等待VM完全启动
        print(f"VM {device_id} created successfully")

    

    def create_all(self):
        """创建所有配置的虚拟机"""
        for device_id, device_config in self.devices.items():
            if 'vm' not in device_config:
                continue
            
            # 检查是否已存在同名VM
            if self.vm_manager.is_vm_exists(device_id):
                print(f"Warning: VM {device_id} already exists, skipping...")
                continue
            
            try:
                self._create_vm(device_id, device_config)
            except Exception as e:
                print(f"Failed to create VM {device_id}: {str(e)}")
                continue

        # TODO: 通过multipass list 获取所有vm的ip

# def main():
#     if len(sys.argv) != 2:
#         print_usage()
    
#     config_file = sys.argv[1]
#     if not os.path.exists(config_file):
#         print(f"Config file not found: {config_file}")
#         sys.exit(1)
    
#     try:
#         creator = VMCreator(config_file)
#         creator.create_all()
#     except Exception as e:
#         print(f"Error: {str(e)}")
#         sys.exit(1)

# if __name__ == "__main__":
#     main()
