import json
import os
import subprocess
import sys

current_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.append(current_dir)
import util
import vm_mgr

id_rsa_path = util.id_rsa_path
ENV_CONFIG = util.ENV_CONFIG
VM_DEVICE_CONFIG = util.VM_DEVICE_CONFIG


class remote_device:
    def __init__(self, device_id: str):
        self.device_id = device_id
        self.remote_port = 22
        self.remote_username = "root"
        
        config = self._load_config()
        if config is None:
            raise Exception("device config not found")
        self.remote_port = config.get('port', 22)
        self.remote_username = config.get('username', 'root')
        self.apps = config.get('apps', {})
        self.config = config

        device_info = self._load_device_info()
        if device_info is None:
            raise Exception("device info not found")
        self.remote_ip = device_info.get('ipv4', ['127.0.0.1'])[0]
        
        # 检测设备类型：如果有 vm 配置，使用 vm_mgr；否则使用 SSH
        self.is_vm = 'vm' in config
        if self.is_vm:
            # 根据 vm 配置选择后端类型，默认为 multipass
            vm_config = config.get('vm', {})
            backend_type = vm_config.get('backend', 'multipass')
            self.vm_manager = vm_mgr.VMManager(backend_type=backend_type)
        else:
            self.vm_manager = None

    def has_app(self, app_name: str):
        return app_name in self.apps
    
    def get_app_config(self, app_name: str):
        return self.apps.get(app_name,)


    def _load_device_info(self):
        print(f"load device info from {VM_DEVICE_CONFIG}")
        try:
            with open(VM_DEVICE_CONFIG, 'r') as f:
                configs = json.load(f)
                return configs.get(self.device_id, {})
        except FileNotFoundError:
            print(f"{VM_DEVICE_CONFIG} not found")
            return None    
        
    def _load_config(self):
        # 先尝试从新配置系统加载（nodes.json）
        try:
            import config_mgr
            config_manager = config_mgr.ConfigManager()
            node_config = config_manager.get_node(self.device_id)
            
            # 转换为旧格式以保持兼容性
            config = {
                'username': 'root',
                'port': 22,
                'zone_id': node_config.get('zone_id', ''),
                'node_id': node_config.get('node_id', self.device_id),
                'vm': {
                    'backend': 'multipass'
                },
                'apps': {}
            }
            
            # 转换 apps 列表为字典格式
            apps_list = node_config.get('apps', [])
            for app_name in apps_list:
                try:
                    app_config = config_manager.get_app(app_name)
                    config['apps'][app_name] = {
                        'start': app_config.get('commands', {}).get('start', ''),
                        'stop': app_config.get('commands', {}).get('stop', '')
                    }
                except ValueError:
                    pass
            
            return config
        except (ImportError, ValueError, FileNotFoundError):
            # 如果新配置系统不可用，回退到旧配置
            pass
        
        # 回退到旧配置系统
        try:
            with open(ENV_CONFIG, 'r') as f:
                configs = json.load(f)
                return configs.get(self.device_id, {})
        except FileNotFoundError:
            print(f"{ENV_CONFIG} not found")
            return None
    
    def pull(self, remote_path, local_path, recursive=False):
        """
        从远程设备拉取文件或目录到本地（通用接口）
        根据设备类型自动选择使用 vm_mgr 或 SSH
        
        Args:
            remote_path: 远程文件或目录路径
            local_path: 本地目标路径
            recursive: 是否递归复制目录
        """
        if self.is_vm and self.vm_manager:
            # 使用 vm_mgr 接口
            success = self.vm_manager.pull_file(self.device_id, remote_path, local_path, recursive)
            if not success:
                raise Exception(f"Failed to pull file from VM {self.device_id}")
        else:
            # 使用 SSH/SCP
            self._scp_pull(remote_path, local_path, recursive)
    
    def push(self, local_path, remote_path, recursive=False):
        """
        推送本地文件或目录到远程设备（通用接口）
        根据设备类型自动选择使用 vm_mgr 或 SSH
        
        Args:
            local_path: 本地文件或目录路径
            remote_path: 远程目标路径
            recursive: 是否递归复制目录
        """
        if self.is_vm and self.vm_manager:
            # 使用 vm_mgr 接口
            success = self.vm_manager.push_file(self.device_id, local_path, remote_path, recursive)
            if not success:
                raise Exception(f"Failed to push file to VM {self.device_id}")
        else:
            # 使用 SSH/SCP
            self._scp_put(local_path, remote_path, recursive)
    
    def _scp_pull(self, remote_path, local_path, recursive=False):
        """
        使用 scp 将远程文件或目录复制到本地（内部方法）
        
        Args:
            remote_path: 远程文件或目录路径
            local_path: 本地目标路径
            recursive: 是否递归复制目录
        """
        scp_command = [
            "scp",
            '-i', id_rsa_path,
        ]
        if recursive:
            scp_command.append("-r")
        
        scp_command.extend([
            f"{self.remote_username}@{self.remote_ip}:{remote_path}",
            local_path
        ])
        
        result = subprocess.run(scp_command, capture_output=True, text=True)
        if result.returncode != 0:
            raise Exception(f"SCP failed: {result.stderr}")

    def _scp_put(self, local_path, remote_path, recursive=False):
        """
        使用 scp 将本地文件或目录复制到远程设备（内部方法）
        
        Args:
            local_path: 本地文件或目录路径
            remote_path: 远程目标路径
            recursive: 是否递归复制目录
        """
        scp_command = [
            "scp",
            '-i', id_rsa_path,
        ]
        if recursive:
            scp_command.append("-r")
        
        scp_command.extend([
            local_path,
            f"{self.remote_username}@{self.remote_ip}:{remote_path}"
        ])
        
        result = subprocess.run(scp_command, capture_output=True, text=True)
        if result.returncode != 0:
            raise Exception(f"SCP failed: {result.stderr}")
    
    def scp_pull(self, remote_path, local_path, recursive=False):
        """
        [已弃用] 使用 scp 将远程文件或目录复制到本地
        请使用 pull() 方法替代
        
        Args:
            remote_path: 远程文件或目录路径
            local_path: 本地目标路径
            recursive: 是否递归复制目录
        """
        import warnings
        warnings.warn("scp_pull() is deprecated, use pull() instead", DeprecationWarning, stacklevel=2)
        self.pull(remote_path, local_path, recursive)

    def scp_put(self, local_path, remote_path, recursive=False):
        """
        [已弃用] 使用 scp 将本地文件或目录复制到远程设备
        请使用 push() 方法替代
        
        Args:
            local_path: 本地文件或目录路径
            remote_path: 远程目标路径
            recursive: 是否递归复制目录
        """
        import warnings
        warnings.warn("scp_put() is deprecated, use push() instead", DeprecationWarning, stacklevel=2)
        self.push(local_path, remote_path, recursive)

    def run_command(self, command: str):
        """
        在远程设备上执行命令
        根据设备类型自动选择使用 vm_mgr 或 SSH
        """
        if self.is_vm and self.vm_manager:
            # 使用 vm_mgr 接口
            print(f"run_command (VM): {command}")
            return self.vm_manager.exec_command(self.device_id, command)
        else:
            # 使用 SSH
            ssh_command = [
                'ssh',
                '-o', 'StrictHostKeyChecking=no',
                '-p', str(self.remote_port),
                '-i', id_rsa_path,
                f"{self.remote_username}@{self.remote_ip}",
                command
            ]
            print(f"run_command: {ssh_command}")
            
            try:
                result = subprocess.run(
                    ssh_command,
                    capture_output=True,
                    text=True,
                    timeout=300  # 5分钟超时
                )
                return result.stdout, result.stderr
            except subprocess.TimeoutExpired:
                return None, "Command execution timed out"
            except Exception as e:
                return None, str(e)

    def get_device_info(self):
        return {
            'device_id': self.device_id,
            'ip': self.remote_ip,
            'port': self.remote_port,
            'username': self.remote_username
        }
