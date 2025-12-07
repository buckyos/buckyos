"""
配置管理器：读取和解析配置文件
"""
import json
import os
from pathlib import Path
import sys

current_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.append(current_dir)
from app_list import AppConfig, AppList
from remote_device import RemoteDeviceInterface, VMInstanceRemoteDevice
from vm_mgr import VMManager, VMNodeList
import util

class Workspace:
    def __init__(self, workspace_dir: Path):
        vm_mgr = VMManager.get_instance()
        vm_mgr.set_template_base_dir(workspace_dir / "templates")
        self.workspace_dir: Path = workspace_dir

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

    def build_env_params_for_node(self, node_id: str,app_params: dict) -> dict:
        return app_params


    def _create_remote_devices_by_vm_instances(self):
        self.remote_devices = {}
        for node_id in self.nodes.get_all_node_ids():
            remote_device = VMInstanceRemoteDevice(node_id)
            self.remote_devices[node_id] = remote_device
        

    def clean_vms(self):
        # 根据workspace中的nodes中的配置，删除vm
        vm_mgr = VMManager.get_instance()
        for node_id,node_config in self.nodes.nodes.items():
            vm_mgr.delete_vm(node_id)

    def create_vms(self):
        # 根据workspace中的nodes中的配置，创建vm
        vm_mgr = VMManager.get_instance()
        for node_id,node_config in self.nodes.nodes.items():
            vm_mgr.create_vm(node_id, node_config)
        

    def snapshot(self, snapshot_name: str):
        # 根据workspace中的nodes中的配置，创建快照
        vm_mgr = VMManager.get_instance()
        for node_id,node_config in self.nodes.nodes.items():
            vm_mgr.snapshot(node_id, snapshot_name)
        

    def restore(self, snapshot_name: str):
        # 根据workspace中的nodes中的配置，恢复快照
        vm_mgr = VMManager.get_instance()
        for node_id,node_config in self.nodes.nodes.items():
            vm_mgr.restore(node_id, snapshot_name)

    def info_vms(self):
        # 根据workspace中的nodes中的配置，查看vm状态
        info = {}
        for node_id,node_config in self.nodes.nodes.items():
            vm_mgr = VMManager.get_instance()
            vm_status = {}
            vm_status["ip_v4"] = vm_mgr.get_ip_v4(node_id)
            info[node_id] = vm_status
        
        return info

    def install(self, device_id: str,app_list:list[str] = None):
        # 根据workspace中的app_list中的配置，向remote_device安装app
        if app_list is None:
            app_list = self.app_list.get_all_app_names()

        remote_device = self.remote_devices[device_id]
        if remote_device is None:
            raise ValueError(f"Remote device '{device_id}' not found")

        for app_name in app_list:
            app_config = self.app_list.get_app(app_name)
            if app_config is None:
                raise ValueError(f"App '{app_name}' not found")
            source_dir = app_config.get_dir("source")
            target_dir = app_config.get_dir("target")
            self.execute_app_command(None, app_name, "build_all")
            
            ## 根据目录设置，将Host上的Source目录的文件推送到remote_device的target目录
            remote_device.push(source_dir, target_dir)
        

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
            target_bin_dir = app_config.get_dir("target_bin")
            self.execute_app_command(None, app_name, "build")
            
            ## 根据目录设置，将Host上的Source目录的文件推送到remote_device的target目录
            remote_device.push(source_bin_dir, target_bin_dir)

    def exec_app_command(self, device_id: str,app_name: str,cmd_name: str ):
        # 根据workspace中的app_list中的配置，向remote_device执行action，执行的内部会调用run
        vm_config = self.nodes.get_node(device_id)
        if vm_config is None:
            raise ValueError(f"Node '{device_id}' not found")
        app_param = self.nodes.get_app_params(device_id, app_name)
        if app_param is None:
            raise ValueError(f"App '{app_name}' not found")
        env_params = self.build_env_params_for_node(device_id, app_param)
        app_config = self.app_list.get_app(app_name)
        if app_config is None:
            raise ValueError(f"App '{app_name}' not found")
        
        command_config = app_config.get_command(cmd_name,env_params)
        if command_config is None:
            raise ValueError(f"Command '{cmd_name}' not found")
        
        self.run(device_id, command_config)

    def run(self, device_id: str, cmds: list[str]):
        if device_id is None:
            for command in cmds:
                os.system(command)
            return

        # 根据workspace中的remote_devices中的配置，向remote_device执行命令
        remote_device = self.remote_devices[device_id]
        if remote_device is None:
            raise ValueError(f"Remote device '{device_id}' not found")
        for command in cmds:
            remote_device.run_command(command)

    def state(self,device_id: str):
        # 根据workspace中的app_list中的配置，查看remote_devices上的app状态（其实是通过执行action来查看）
        pass

    def clog(self):
        # 根据workspace中的remote_devices中的配置，收集remote_devices上的日志到本地
        pass




        