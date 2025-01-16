class remote_device:
    def __init__(self, device_id: str):
        import json
        import os
        
        self.device_id = device_id
        self.config = self._load_config()
        
    def _load_config(self):
        import json
        # 假设配置文件在 ~/.remote_devices/config.json
        config_path = os.path.expanduser('~/.remote_devices/config.json')
        try:
            with open(config_path, 'r') as f:
                configs = json.load(f)
                return configs.get(self.device_id, {})
        except FileNotFoundError:
            return {}

    def run_command(self, command: str):
        import subprocess
        
        ssh_command = [
            'ssh',
            '-o', 'StrictHostKeyChecking=no',
            '-p', str(self.config.get('port', 22)),
            f"{self.config.get('username')}@{self.config.get('hostname')}",
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
            'hostname': self.config.get('hostname'),
            'username': self.config.get('username'),
            'port': self.config.get('port', 22)
        }
