import json
import os
import subprocess
        
class remote_device:
    def __init__(self, device_id: str):
        self.device_id = device_id
        self.remote_port = 22
        self.remote_username = "root"
        
        config = self._load_config()
        self.remote_port = config.get('port', 22)
        self.remote_username = config.get('username', 'root')

        device_info = self._load_device_info()
        self.remote_ip = device_info.get('ipv4', ['127.0.0.1'])[0]

    def _load_device_info(self):
        # 假设配置文件在 ~/.remote_devices/config.json
        config_path = os.path.expanduser('device_info.json')
        try:
            with open(config_path, 'r') as f:
                configs = json.load(f)
                return configs.get(self.device_id, {})
        except FileNotFoundError:
            return {}    
    def _load_config(self):
        # 假设配置文件在 ~/.remote_devices/config.json
        config_path = os.path.expanduser('env_config.json')
        try:
            with open(config_path, 'r') as f:
                configs = json.load(f)
                return configs.get(self.device_id, {})
        except FileNotFoundError:
            return {}
        
    def scp_put(self, local_path, remote_path):
        scp_command = f"scp {local_path} {self.remote_username}@{self.remote_ip}:{remote_path}"
        subprocess.run(scp_command, shell=True, check=True)

    def run_command(self, command: str):

        ssh_command = [
            'ssh',
            '-o', 'StrictHostKeyChecking=no',
            '-p', str(self.remote_port),
            f"{self.remote_username}@{self.remote_ip}",
            command
        ]
        
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
