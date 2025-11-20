"""
虚拟机管理层抽象
支持多种后端实现（multipass、docker、kvm等）
"""
import subprocess
import abc
import os
import sys

current_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.append(current_dir)
import util


class VMBackend(abc.ABC):
    """虚拟机后端抽象基类"""
    
    @abc.abstractmethod
    def create_vm(self, vm_name: str, config: dict) -> bool:
        """
        创建虚拟机
        
        Args:
            vm_name: 虚拟机名称
            config: 虚拟机配置（cpu, memory, disk等）
        
        Returns:
            bool: 是否创建成功
        """
        pass
    
    @abc.abstractmethod
    def delete_vm(self, vm_name: str) -> bool:
        """
        删除虚拟机
        
        Args:
            vm_name: 虚拟机名称
        
        Returns:
            bool: 是否删除成功
        """
        pass
    
    @abc.abstractmethod
    def exec_command(self, vm_name: str, command: str) -> tuple:
        """
        在虚拟机中执行命令
        
        Args:
            vm_name: 虚拟机名称
            command: 要执行的命令
        
        Returns:
            tuple: (stdout, stderr)
        """
        pass
    
    @abc.abstractmethod
    def push_file(self, vm_name: str, local_path: str, remote_path: str, recursive: bool = False) -> bool:
        """
        将本地文件推送到虚拟机
        
        Args:
            vm_name: 虚拟机名称
            local_path: 本地文件路径
            remote_path: 远程文件路径
            recursive: 是否递归复制目录
        
        Returns:
            bool: 是否成功
        """
        pass
    
    @abc.abstractmethod
    def pull_file(self, vm_name: str, remote_path: str, local_path: str, recursive: bool = False) -> bool:
        """
        从虚拟机拉取文件到本地
        
        Args:
            vm_name: 虚拟机名称
            remote_path: 远程文件路径
            local_path: 本地文件路径
            recursive: 是否递归复制目录
        
        Returns:
            bool: 是否成功
        """
        pass
    
    @abc.abstractmethod
    def get_vm_ip(self, vm_name: str) -> list:
        """
        获取虚拟机的IP地址
        
        Args:
            vm_name: 虚拟机名称
        
        Returns:
            list: IP地址列表
        """
        pass
    
    @abc.abstractmethod
    def is_vm_exists(self, vm_name: str) -> bool:
        """
        检查虚拟机是否存在
        
        Args:
            vm_name: 虚拟机名称
        
        Returns:
            bool: 是否存在
        """
        pass


class MultipassVMBackend(VMBackend):
    """Multipass 虚拟机后端实现"""
    
    def __init__(self):
        self.id_rsa_path = util.id_rsa_path
        self.remote_username = "root"
    
    def create_vm(self, vm_name: str, config: dict) -> bool:
        """使用 multipass 创建虚拟机"""
        cpu = config.get('cpu', 1)
        memory = config.get('memory', '1G')
        disk = config.get('disk', '5G')
        config_base = config.get('config_base', '')
        
        cmd = f"multipass launch --name {vm_name} --cpus {cpu} --memory {memory} --disk {disk}"
        
        # 如果提供了 config_base，添加 cloud-init 配置
        if config_base:
            init_yaml = os.path.join(config_base, 'vm_init.yaml')
            if os.path.exists(init_yaml):
                cmd += f" --cloud-init {init_yaml}"
        
        try:
            result = subprocess.run(
                cmd, shell=True, check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True
            )
            # 等待 VM 启动
            import time
            time.sleep(3)
            # 设置 hostname
            stdout, stderr = self.exec_command(vm_name, f"sudo hostnamectl set-hostname {vm_name}")
            if stderr:
                print(f"Warning: Failed to set hostname: {stderr}")
            return True
        except subprocess.CalledProcessError as e:
            print(f"Failed to create VM {vm_name}: {e.stderr}")
            return False
    
    def delete_vm(self, vm_name: str) -> bool:
        """使用 multipass 删除虚拟机"""
        try:
            result = subprocess.run(
                ["multipass", "delete", vm_name],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True
            )
            # 执行 purge 以彻底删除
            subprocess.run(
                ["multipass", "purge"],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True
            )
            return True
        except subprocess.CalledProcessError as e:
            print(f"Failed to delete VM {vm_name}: {e.stderr}")
            return False
    
    def exec_command(self, vm_name: str, command: str) -> tuple:
        """使用 multipass exec 执行命令"""

        try:
            result = subprocess.run(
                ["multipass", "exec", vm_name, "--", "bash", "-c", command],
                capture_output=True,
                text=True,
                timeout=300
            )
            return result.stdout, result.stderr
        except subprocess.TimeoutExpired:
            return None, "Command execution timed out"
        except Exception as e:
            return None, str(e)
    
    def push_file(self, vm_name: str, local_path: str, remote_path: str, recursive: bool = False) -> bool:
        """使用 multipass transfer 推送文件"""
        try:
            cmd = ["multipass", "transfer"]
            if recursive:
                cmd.append("-r")
            cmd.extend([local_path, f"{vm_name}:{remote_path}"])
            
            result = subprocess.run(
                cmd,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True
            )
            return True
        except subprocess.CalledProcessError as e:
            print(f"Failed to push file to {vm_name}: {e.stderr}")
            return False
    
    def pull_file(self, vm_name: str, remote_path: str, local_path: str, recursive: bool = False) -> bool:
        """使用 multipass transfer 拉取文件"""
        try:
            cmd = ["multipass", "transfer"]
            if recursive:
                cmd.append("-r")
            cmd.extend([f"{vm_name}:{remote_path}", local_path])
            
            result = subprocess.run(
                cmd,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True
            )
            return True
        except subprocess.CalledProcessError as e:
            print(f"Failed to pull file from {vm_name}: {e.stderr}")
            return False
    
    def get_vm_ip(self, vm_name: str) -> list:
        """获取虚拟机的IP地址"""
        return util.get_multipass_ip(vm_name)
    
    def is_vm_exists(self, vm_name: str) -> bool:
        """检查虚拟机是否存在"""
        try:
            result = subprocess.run(
                ["multipass", "list"],
                capture_output=True,
                text=True,
                check=True
            )
            return vm_name in result.stdout
        except subprocess.CalledProcessError:
            return False


class VMManager:
    """虚拟机管理器，根据配置选择后端"""
    
    def __init__(self, backend_type: str = "multipass"):
        """
        初始化虚拟机管理器
        
        Args:
            backend_type: 后端类型，目前支持 "multipass"
        """
        if backend_type == "multipass":
            self.backend = MultipassVMBackend()
        else:
            raise ValueError(f"Unsupported backend type: {backend_type}")
    
    def create_vm(self, vm_name: str, config: dict) -> bool:
        """创建虚拟机"""
        return self.backend.create_vm(vm_name, config)
    
    def delete_vm(self, vm_name: str) -> bool:
        """删除虚拟机"""
        return self.backend.delete_vm(vm_name)
    
    def exec_command(self, vm_name: str, command: str) -> tuple:
        """在虚拟机中执行命令"""
        return self.backend.exec_command(vm_name, command)
    
    def push_file(self, vm_name: str, local_path: str, remote_path: str, recursive: bool = False) -> bool:
        """推送文件到虚拟机"""
        return self.backend.push_file(vm_name, local_path, remote_path, recursive)
    
    def pull_file(self, vm_name: str, remote_path: str, local_path: str, recursive: bool = False) -> bool:
        """从虚拟机拉取文件"""
        return self.backend.pull_file(vm_name, remote_path, local_path, recursive)
    
    def get_vm_ip(self, vm_name: str) -> list:
        """获取虚拟机IP"""
        return self.backend.get_vm_ip(vm_name)
    
    def is_vm_exists(self, vm_name: str) -> bool:
        """检查虚拟机是否存在"""
        return self.backend.is_vm_exists(vm_name)

