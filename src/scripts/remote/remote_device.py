import json
import os
import subprocess
        
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

        device_info = self._load_device_info()
        if device_info is None:
            raise Exception("device info not found")
        self.remote_ip = device_info.get('ipv4', ['127.0.0.1'])[0]

    def has_app(self, app_name: str):
        return app_name in self.apps
    
    def get_app_config(self, app_name: str):
        return self.apps.get(app_name,)


    def _load_device_info(self):
        # 配置文件在 ~/.buckyos_dev/device_info.json
        config_path = os.path.expanduser('~/.buckyos_dev/device_info.json')
        try:
            with open(config_path, 'r') as f:
                configs = json.load(f)
                return configs.get(self.device_id, {})
        except FileNotFoundError:
            print("~/.buckyos_dev/device_info.json not found")
            return None    
        
    def _load_config(self):
        # 配置文件在 ~/.buckyos_dev/env_config.json
        config_path = os.path.expanduser('~/.buckyos_dev/env_config.json')
        #print(f"loading config from {config_path}")
        try:
            with open(config_path, 'r') as f:
                configs = json.load(f)
                return configs.get(self.device_id, {})
        except FileNotFoundError:
            print("~/.buckyos_dev/env_config.json not found")
            return None
    
    def scp_pull(self, remote_path, local_path, recursive=False):
        """
        使用 scp 将远程文件或目录复制到本地
        
        Args:
            remote_path: 远程文件或目录路径
            local_path: 本地目标路径
            recursive: 是否递归复制目录
        """
        scp_command = [
            "scp",
            '-i', os.path.expanduser('~/.buckyos_dev/id_rsa'),
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

    def scp_put(self, local_path, remote_path, recursive=False):
        """
        使用 scp 将本地文件或目录复制到远程设备
        
        Args:
            local_path: 本地文件或目录路径
            remote_path: 远程目标路径
            recursive: 是否递归复制目录
        """
        scp_command = [
            "scp",
            '-i', os.path.expanduser('~/.buckyos_dev/id_rsa'),
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

    def run_command(self, command: str):

        ssh_command = [
            'ssh',
            '-o', 'StrictHostKeyChecking=no',
            '-p', str(self.remote_port),
            '-i', os.path.expanduser('~/.buckyos_dev/id_rsa'),
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
