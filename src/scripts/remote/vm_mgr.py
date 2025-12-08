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
import re


current_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.append(current_dir)



class VMConfig:
    """虚拟机配置"""
    def __init__(self, node_id: str):
        self.node_id : str = node_id
        self.vm_template : str = None
        self.vm_params : dict = None
        self.network : dict = None
        #app_name -> app_instance params
        self.apps : dict [str,dict]= None
        self.init_commands : list[str] = None # init_comands会在创建vm后立刻执行
        self.instance_commands : list[str] = None # instance_commands会在所有的instance创建完成后，按instance_order顺序执行，因此可以在命令行中引用已创建的vm instance的属性

    def get_dir(self, dir_name: str) -> str:
        return self.directories.get(dir_name, None)


class VMNodeList:
    def __init__(self):
        self.nodes : dict [str, VMConfig]= None
        self.instance_order : list[str] = None
    def get_node(self, node_id: str) -> VMConfig:
        return self.nodes.get(node_id)
    
    def get_all_node_ids(self) -> list[str]:
        return list(self.nodes.keys())

    def get_node_id_by_init_orders(self) -> list[str]:
        return self.instance_order

    def get_app_params(self, node_id: str, app_name: str) -> dict:
        return self.nodes.get(node_id).apps.get(app_name, None)

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
            vm_cfg.instance_commands = cfg.get("instance_commands", [])
            vm_cfg.directories = cfg.get("directories", {})
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
    def push_dir(self, vm_name: str, local_dir: str, remote_dir: str) -> bool:
        """
        将本地目录递归推送到虚拟机目录
        """
        pass

    @abc.abstractmethod
    def pull_dir(self, vm_name: str, remote_dir: str, local_dir: str) -> bool:
        """
        将虚拟机目录递归拉取到本地目录
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
        print(f"create vm {vm_name} with config {config}")
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

            print(f"create vm {vm_name} success")    
            return True
        except subprocess.CalledProcessError as e:
            print(f"Failed to create VM {vm_name}: {e.stderr}")
            return False
    
    def delete_vm(self, vm_name: str) -> bool:
        """使用 multipass 删除虚拟机"""
        print(f"delete vm {vm_name} ...")
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
            print(f"delete vm {vm_name} success")
            return True
        except subprocess.CalledProcessError as e:
            print(f"Failed to delete VM {vm_name}: {e.stderr}")
            return False
    
    def exec_command(self, vm_name: str, command: str) -> tuple:
        """使用 multipass exec 执行命令"""
        print(f"exec [ {command} ] on vm {vm_name} ...")
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
            print(f"push file {local_path} to {vm_name}:{remote_path} ...")
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
            print(f"pull file {vm_name}:{remote_path} to {local_path} ...")
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
    
    def get_vm_ip(self, vm_name: str) -> list[str]:
        """获取虚拟机的IP地址"""
        try:
            result = subprocess.run(
                ["multipass", "info", vm_name],
                capture_output=True,
                text=True,
                check=True,
            )
            # 匹配 IPv4 列表，兼容多行
            ip_pattern = r"IPv4:\s+((?:\d+\.\d+\.\d+\.\d+\s*)+)"
            match = re.search(ip_pattern, result.stdout)
            if match:
                ips = [ip.strip() for ip in match.group(1).split()]
                if ips:
                    return ips
            raise RuntimeError(f"No IPv4 address found for VM {vm_name}")
        except subprocess.CalledProcessError as e:
            print(f"Failed to get IP for VM {vm_name}: {e.stderr}")
            raise
        except Exception as e:
            print(f"Unknown error getting IP for VM {vm_name}: {e}")
            raise

    
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
            print(f"create snapshot {snapshot_name} on vm {node_id} ...")
            subprocess.run(
                ["multipass", "stop", node_id],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            subprocess.run(
                ["multipass", "snapshot", node_id, "--name", snapshot_name],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            subprocess.run(
                ["multipass", "start", node_id],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            print(f"create snapshot {snapshot_name} on vm {node_id} success")
            return True
        except subprocess.CalledProcessError as e:
            print(f"Failed to snapshot VM {node_id}: {e.stderr}")
            return False
    
    def restore(self, node_id: str, snapshot_name: str) -> bool:
        """恢复快照"""
        try:
            print(f"restore vm {node_id} to snapshot {snapshot_name} ...")
            subprocess.run(
                ["multipass", "stop", node_id],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            subprocess.run(
                # multipass restore 需要 "<instance>.<snapshot>" 作为单一参数
                ["multipass", "restore", f"{node_id}.{snapshot_name}","-d"],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            subprocess.run(
                ["multipass", "start", node_id],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            print(f"restore vm {node_id} to snapshot  {snapshot_name} success")
            return True
        except subprocess.CalledProcessError as e:
            print(f"Failed to restore VM {node_id} snapshot {snapshot_name}: {e.stderr}")
            return False
    
    def set_template_base_dir(self, template_base_dir: Path) -> bool:
        """设置模板基础目录"""
        self.template_base_dir = template_base_dir
        return True
    
    def push_dir(self, vm_name: str, local_dir: str, remote_dir: str) -> bool:
        """递归推送目录（multipass transfer -r）"""
        try:
            # 确保远端目标目录存在
            self.exec_command(vm_name, f"mkdir -p {remote_dir}")
            return self.push_file(vm_name, local_dir, remote_dir, recursive=True)
        except Exception as e:
            print(f"Failed to push dir to {vm_name}: {e}")
            return False

    def pull_dir(self, vm_name: str, remote_dir: str, local_dir: str) -> bool:
        """递归拉取目录（multipass transfer -r）"""
        try:
            # 确保本地目标目录存在
            os.makedirs(local_dir, exist_ok=True)
            return self.pull_file(vm_name, remote_dir, local_dir, recursive=True)
        except Exception as e:
            print(f"Failed to pull dir from {vm_name}: {e}")
            return False
    
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

    def push_dir(self, vm_name: str, local_dir: str, remote_dir: str) -> bool:
        """递归推送目录"""
        return self.backend.push_dir(vm_name, local_dir, remote_dir)

    def pull_dir(self, vm_name: str, remote_dir: str, local_dir: str) -> bool:
        """递归拉取目录"""
        return self.backend.pull_dir(vm_name, remote_dir, local_dir)
    
    def get_vm_ip(self, vm_name: str) -> list:
        """获取虚拟机IP"""
        return self.backend.get_vm_ip(vm_name)
    
    def is_vm_exists(self, vm_name: str) -> bool:
        """检查虚拟机是否存在"""
        return self.backend.is_vm_exists(vm_name)

