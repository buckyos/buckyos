"""
虚拟机管理层抽象
支持多种后端实现（multipass、docker、kvm等）
"""
from pathlib import Path
import subprocess
import abc
import os
import sys
import json

current_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.append(current_dir)
import util


class VMConfig:
    """虚拟机配置"""
    def __init__(self, node_id: str):
        self.node_id : str = node_id
        self.vm_template : str = None
        self.vm_params : dict = None
        self.network : dict = None
        #app_name -> app_instance params
        self.apps : dict [str,dict]= None
        self.init_commands : list[str] = None # init_comands会在所有的instance创建完成后执行,因此可以在命令行中引用一些vm instance的属性

class VMNodeList:
    def __init__(self):
        self.nodes : dict [str, VMConfig]= None
        self.instance_order : list[str] = None
    def get_node(self, node_id: str) -> VMConfig:
        return self.nodes.get(node_id)
    
    def get_all_node_ids(self) -> list[str]:
        return list(self.nodes.keys())

    def load_from_file(self, file_path: Path):
        with open(file_path, "r") as f:
            data = json.load(f)

        self.nodes = {}
        self.instance_order = data.get("instance_order", [])

        for node_id, cfg in data.get("nodes", {}).items():
            vm_cfg = VMConfig(node_id=cfg.get("node_id", node_id))
            vm_cfg.vm_template = cfg.get("vm_template")
            vm_cfg.vm_params = cfg.get("vm_params", {})
            vm_cfg.network = cfg.get("network", {})
            vm_cfg.apps = cfg.get("apps", {})
            vm_cfg.init_commands = cfg.get("init_commands", [])
            self.nodes[node_id] = vm_cfg

        return self


class VMBackend(abc.ABC):
    """虚拟机后端抽象基类"""
    
    @abc.abstractmethod
    def create_vm(self, vm_name: str, config: VMConfig) -> bool:
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

    @abc.abstractmethod
    def snapshot(self, node_id: str, snapshot_name: str) -> bool:
        """创建快照"""
        pass
    
    @abc.abstractmethod
    def restore(self, node_id: str, snapshot_name: str) -> bool:
        """恢复快照"""
        pass

    @abc.abstractmethod
    def set_template_base_dir(self, template_base_dir: Path) -> bool:
        """设置模板基础目录"""
        pass

class MultipassVMBackend(VMBackend):
    """Multipass 虚拟机后端实现"""
    
    def __init__(self):
        self.id_rsa_path = util.id_rsa_path
        self.remote_username = "root"
        self.template_base_dir: Path = None
    
    def create_vm(self, vm_name: str, config: VMConfig) -> bool:
        """使用 multipass 创建虚拟机"""
        cpu = config.vm_params.get('cpu', 1)
        memory = config.vm_params.get('memory', '1G')
        disk = config.vm_params.get('disk', '5G')
        template_name = config.vm_template
        
        cmd = f"multipass launch --name {vm_name} --cpus {cpu} --memory {memory} --disk {disk}"
        
        # 如果提供了 config_base，添加 cloud-init 配置
        if template_name:
            init_yaml = os.path.join(self.template_base_dir, f'{template_name}.yaml')
            cmd += f" --cloud-init {init_yaml}"
        print(cmd)
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

    def snapshot(self, node_id: str, snapshot_name: str) -> bool:
        """创建快照"""
        try:
            subprocess.run(
                ["multipass", "snapshot", node_id, snapshot_name],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            return True
        except subprocess.CalledProcessError as e:
            print(f"Failed to snapshot VM {node_id}: {e.stderr}")
            return False
    
    def restore(self, node_id: str, snapshot_name: str) -> bool:
        """恢复快照"""
        try:
            subprocess.run(
                ["multipass", "restore", node_id, snapshot_name],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            return True
        except subprocess.CalledProcessError as e:
            print(f"Failed to restore VM {node_id} snapshot {snapshot_name}: {e.stderr}")
            return False
    
    def set_template_base_dir(self, template_base_dir: Path) -> bool:
        """设置模板基础目录"""
        self.template_base_dir = template_base_dir
        return True
    
class VMManager:
    """虚拟机管理器，根据配置选择后端（单例）"""

    _instance = None
    _backend_type = None

    def __new__(cls, backend_type: str = "multipass"):
        if cls._instance is None:
            cls._instance = super().__new__(cls)
        return cls._instance

    def __init__(self, backend_type: str = "multipass"):
        """
        初始化虚拟机管理器（单例），后端类型首次创建时确定
        
        Args:
            backend_type: 后端类型，目前支持 "multipass"
        """
        if getattr(self, "_initialized", False):
            # 后续初始化保持后端一致
            if backend_type != self._backend_type:
                raise ValueError(
                    f"VMManager is singleton with backend '{self._backend_type}', "
                    f"got '{backend_type}'"
                )
            return

        if backend_type == "multipass":
            self.backend = MultipassVMBackend()
        else:
            raise ValueError(f"Unsupported backend type: {backend_type}")

        VMManager._backend_type = backend_type
        self._initialized = True

    @classmethod
    def get_instance(cls):
        """获取单例实例"""
        return cls("multipass")

    @classmethod
    def get_backend_type(cls):
        """获取当前单例后端类型"""
        return cls._backend_type or "multipass"

    def set_template_base_dir(self, template_base_dir: Path):
        """设置模板基础目录"""
        self.backend.set_template_base_dir(template_base_dir)
    
    def snapshot(self, node_id: str, snapshot_name: str) -> bool:
        """创建快照"""
        return self.backend.snapshot(node_id, snapshot_name)
    
    def restore(self, node_id: str, snapshot_name: str) -> bool:
        """恢复快照"""
        return self.backend.restore(node_id, snapshot_name)
    
    def create_vm(self, vm_name: str, config: VMConfig) -> bool:
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

