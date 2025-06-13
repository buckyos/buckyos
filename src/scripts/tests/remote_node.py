import subprocess


class RemoteNode:
    def __init__(self, remote_ip: str, identity_file: str, remote_port: int = 22, remote_username: str = "root"):
        self.remote_port = remote_port
        self.remote_username = remote_username
        self.remote_ip = remote_ip
        self.identity_file = identity_file


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
            '-i', self.identity_file,
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
            '-i', self.identity_file,
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
            '-i', self.identity_file,
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
            'ip': self.remote_ip,
            'port': self.remote_port,
            'username': self.remote_username
        }
