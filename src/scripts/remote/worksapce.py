"""
配置管理器：读取和解析配置文件
"""
import re
import os
import shutil
import tempfile
from pathlib import Path
import sys
import time
import platform
from typing import Optional

current_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.append(current_dir)
from app_list import AppConfig, AppList
from remote_device import RemoteDeviceInterface, VMInstanceRemoteDevice
from vm_mgr import VMManager, VMNodeList


def get_temp_dir() -> Path:
    """Return a temporary directory path as Path."""
    return Path(tempfile.gettempdir())


class Workspace:
    _VARIABLE_PATTERN = re.compile(r'\{\{(\w+)\.(\w+)\}\}')
    def __init__(self, workspace_dir: Path,base_dir:Optional[Path] = None):
        vm_mgr = VMManager.get_instance()
        vm_mgr.set_template_base_dir(workspace_dir / "templates")
        self.workspace_dir: Path = workspace_dir
        self.base_dir: Path = base_dir
        if self.base_dir is None:
            self.base_dir = Path(current_dir).parent.parent

        print(f"base_dir: {self.base_dir}")

        self.nodes: VMNodeList= None
        self.remote_devices: dict [str, RemoteDeviceInterface]= None
        self.app_list : AppList= None

    def load(self):
        # 加载workspace中的配置
        nodes_config_path = self.workspace_dir / "nodes.json"
        self.nodes = VMNodeList()
        self.nodes.load_from_file(nodes_config_path)

        app_dir = self.workspace_dir / "apps"
        self.app_list = AppList(app_dir)
        self.app_list.load_app_list()

        self._create_remote_devices_by_vm_instances()

    def build_env_params(self,parent_env_params: Optional[dict] = None) -> dict:
        env_params = {}
        if parent_env_params is not None:
            env_params.update(parent_env_params)
        # 获得所有的环境变量
        system_env_params = os.environ.copy()
        system_env_params["base_dir"] = str(self.base_dir)
        env_params["system"] = system_env_params
        # 根据self.nodes中的配置，构建env_params
        for node_id in self.nodes.get_all_node_ids():
            device_info = self.remote_devices[node_id].get_device_info()
            env_params[node_id] = device_info

        return env_params


    def _create_remote_devices_by_vm_instances(self):
        self.remote_devices = {}
        for node_id in self.nodes.get_all_node_ids():
            remote_device = VMInstanceRemoteDevice(node_id)
            self.remote_devices[node_id] = remote_device
        
    
    def clean_vms(self):#ok
        # 根据workspace中的nodes中的配置，删除vm
        vm_mgr = VMManager.get_instance()
        for node_id,node_config in self.nodes.nodes.items():
            vm_mgr.delete_vm(node_id)

    def create_vms(self):#ok
        # 根据workspace中的nodes中的配置，创建vm
        vm_mgr = VMManager.get_instance()
        for node_id,node_config in self.nodes.nodes.items():
            vm_mgr.create_vm(node_id, node_config)
        # 所有的vm创建完成，按顺序调用init_commands
        print(f"all nodes created, call instance_commands after 10 seconds...")
        time.sleep(10)
        env_params = self.build_env_params()
        print(f"env_params: {env_params}")
        instance_order = self.nodes.get_node_id_by_init_orders()

        for node_id in instance_order:
            node_config = self.nodes.get_node(node_id)
            if node_config is None:
                continue
            if node_config.instance_commands is None:
                continue
            for command in node_config.instance_commands:
                self.run(node_id, [command], env_params)

    def snapshot(self, snapshot_name: str):#ok
        # 根据workspace中的nodes中的配置，创建快照
        vm_mgr = VMManager.get_instance()
        for node_id,node_config in self.nodes.nodes.items():
            print(f"create snapshot {snapshot_name} for node: {node_id}")
            vm_mgr.snapshot(node_id, snapshot_name)
        

    def restore(self, snapshot_name: str):#ok
        # 根据workspace中的nodes中的配置，恢复快照
        vm_mgr = VMManager.get_instance()
        for node_id,node_config in self.nodes.nodes.items():
            vm_mgr.restore(node_id, snapshot_name)

    def info_vms(self):#ok
        # 根据workspace中的nodes中的配置，查看vm状态
        info = {}
        for node_id,node_config in self.nodes.nodes.items():
            vm_mgr = VMManager.get_instance()
            vm_status = {}
            vm_status["ip_v4"] = vm_mgr.get_vm_ip(node_id)
            info[node_id] = vm_status

        env_params = self.build_env_params()
        print(f"env_params: {env_params}")
        
        return info

    def install(self, device_id: str,app_list:list[str] = None):
        # 根据workspace中的app_list中的配置，向remote_device安装app
        if app_list is None:
            app_list = self.app_list.get_all_app_names()
        print(f"install apps to device: {device_id} with apps: {app_list}")
        remote_device = self.remote_devices[device_id]
        if remote_device is None:
            raise ValueError(f"Remote device '{device_id}' not found")

        for app_name in app_list:
            if not self.nodes.have_app(device_id, app_name):
                print(f"App '{app_name}' not found in node: {device_id}, SKIP")
                continue

            app_config = self.app_list.get_app(app_name)
            if app_config is None:
                raise ValueError(f"App '{app_name}' not found")
            source_dir = app_config.get_dir("source")
            source_dir_path = Path(source_dir);
            if not source_dir_path.is_absolute():
                source_dir_path = self.base_dir / source_dir_path
            target_dir = app_config.get_dir("target")
            self.execute_app_command(device_id, app_name, "build_all",True)
            
            ## 根据目录设置，将Host上的Source目录的文件推送到remote_device的target目录
            remote_device.push(source_dir, target_dir)
            self.execute_app_command(device_id, app_name, "install")
        

    def update(self, device_id: str,app_list:list[str] = None):
        # 根据workspace中的app_list中的配置，向remote_device更新app
        # 
        ## 先在host上运行 build 脚本
        ## 根据目录设置，将Host上的source_bin目录的文件推送到remote_device的target_bin目录
        if app_list is None:
            app_list = self.app_list.get_all_app_names()

        remote_device = self.remote_devices[device_id]
        if remote_device is None:
            raise ValueError(f"Remote device '{device_id}' not found")

        for app_name in app_list:
            app_config = self.app_list.get_app(app_name)
            if app_config is None:
                raise ValueError(f"App '{app_name}' not found")
            source_bin_dir = app_config.get_dir("source_bin")
            if source_bin_dir is None:
                print(f"App '{app_name}' not found source_bin_dir, SKIP update")
                continue
            source_bin_dir_path = Path(source_bin_dir);
            if not source_bin_dir_path.is_absolute():
                source_bin_dir_path = self.base_dir / source_bin_dir_path
            target_bin_dir = app_config.get_dir("target_bin")
            self.execute_app_command(device_id, app_name, "build",True)
            
            ## 根据目录设置，将Host上的Source目录的文件推送到remote_device的target目录
            remote_device.push(source_bin_dir, target_bin_dir)
            self.execute_app_command(device_id, app_name, "update")

    def execute_app_command(self, device_id: Optional[str],app_name: str,cmd_name: str ,run_in_host: bool = False):#ok
        # 根据workspace中的app_list中的配置，向remote_device执行action，执行的内部会调用run
        vm_config = self.nodes.get_node(device_id)
        if vm_config is None:
            raise ValueError(f"Node '{device_id}' not found")
        app_param = self.nodes.get_app_params(device_id, app_name)
        if app_param is None:
            raise ValueError(f"App '{app_name}' not found")
        app_env_params = {
            app_name: app_param
        }
        env_params = self.build_env_params(app_env_params)
        app_config = self.app_list.get_app(app_name)
        if app_config is None:
            raise ValueError(f"App '{app_name}' not found")
        
        command_config = app_config.get_command(cmd_name)
        if command_config is None:
            raise ValueError(f"Command '{cmd_name}' not found")
        
        if run_in_host:
            self.run(None, command_config, env_params)
        else:
            self.run(device_id, command_config, env_params)

    def resolve_string(self, text: str,env_params: dict) -> str:#ok
        """
        解析字符串中的所有变量引用
        
        Args:
            text: 包含变量引用的字符串
            env_params: 环境参数
        
        Returns:
            str: 解析后的字符串
        """
        def replace_var(match):
            obj_id = match.group(1)
            attr = match.group(2)
            sub_obj = env_params.get(obj_id)
            if sub_obj is None:
                raise ValueError(f"Object '{obj_id}' not found in env_params")
            value = sub_obj.get(attr)
            if value is None:
                raise ValueError(f"Attribute '{attr}' not found in object '{obj_id}'")
            return value
        
        return self._VARIABLE_PATTERN.sub(replace_var, text)

    def run(self, device_id: Optional[str], cmds: list[str], env_params: dict):
        remote_device = None
        if device_id is not None:
            remote_device = self.remote_devices[device_id]
            if remote_device is None:
                raise ValueError(f"Remote device '{device_id}' not found")
        else:
            device_id = "localhost"

        for command in cmds:
            new_command = self.resolve_string(command, env_params)
            print(f"run resolved command: [ {new_command} ] on {device_id}")
            if remote_device is None:
                os.system(new_command)
            else:
                remote_device.run_command(new_command)

    def state(self,device_id: str):
        # 根据workspace中的app_list中的配置，查看remote_devices上的app状态（其实是通过执行action来查看）
        pass

    def clog(self,target_dir: Optional[Path] = None):#ok
        # 根据workspace中的remote_devices中的配置，收集remote_devices上的日志到本地
        # 收集node里定义的系统日志目录，到本地
        if target_dir is None:
            if platform.system() == "Windows":
                target_dir = get_temp_dir().joinpath("clogs")
            else:
                target_dir = Path("/tmp/clogs")
        
        #remove target dir
        if target_dir.exists():
            shutil.rmtree(target_dir)
            
        for node_id in self.nodes.get_all_node_ids():
            node_config = self.nodes.get_node(node_id)
            if node_config is None:
                continue
            logs_dir = node_config.get_dir("logs")
            if logs_dir is None:
                continue
            real_target_dir = target_dir.joinpath(node_id)
            real_target_dir.mkdir(parents=True, exist_ok=True)
            print(f"collect logs from {node_id}:{logs_dir} to {real_target_dir} ...")
            self.remote_devices[node_id].pull(logs_dir, real_target_dir, True)
            print(f"collect logs from {node_id}:{logs_dir} to {real_target_dir} done")
        




        