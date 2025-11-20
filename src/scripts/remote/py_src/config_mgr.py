"""
配置管理器：读取和解析配置文件
"""
import json
import os
import sys

current_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.append(current_dir)
import util


class ConfigManager:
    """配置管理器"""
    
    def __init__(self, config_base: str = None):
        """
        初始化配置管理器
        
        Args:
            config_base: 配置文件基础目录，默认为 util.CONFIG_BASE
        """
        if config_base is None:
            config_base = util.CONFIG_BASE
        
        self.config_base = config_base
        self.vm_config_path = os.path.join(config_base, "vm_config.json")
        self.app_list_path = os.path.join(config_base, "app_list.json")
        self.nodes_path = os.path.join(config_base, "nodes.json")
        
        self._vm_config = None
        self._app_list = None
        self._nodes = None
    
    def load_vm_config(self):
        """加载 VM 模板配置"""
        if self._vm_config is None:
            with open(self.vm_config_path, 'r') as f:
                self._vm_config = json.load(f)
        return self._vm_config
    
    def load_app_list(self):
        """加载软件列表配置"""
        if self._app_list is None:
            with open(self.app_list_path, 'r') as f:
                self._app_list = json.load(f)
        return self._app_list
    
    def load_nodes(self):
        """加载节点配置"""
        if self._nodes is None:
            with open(self.nodes_path, 'r') as f:
                self._nodes = json.load(f)
        return self._nodes
    
    def get_vm_template(self, template_name: str):
        """
        获取 VM 模板
        
        Args:
            template_name: 模板名称
        
        Returns:
            dict: VM 模板配置
        """
        vm_config = self.load_vm_config()
        templates = vm_config.get("templates", {})
        if template_name not in templates:
            raise ValueError(f"VM template '{template_name}' not found")
        return templates[template_name]
    
    def get_app(self, app_name: str):
        """
        获取软件配置
        
        Args:
            app_name: 软件名称
        
        Returns:
            dict: 软件配置
        """
        app_list = self.load_app_list()
        apps = app_list.get("apps", {})
        if app_name not in apps:
            raise ValueError(f"App '{app_name}' not found")
        return apps[app_name]
    
    def get_node(self, node_id: str):
        """
        获取节点配置
        
        Args:
            node_id: 节点 ID
        
        Returns:
            dict: 节点配置
        """
        nodes = self.load_nodes()
        node_configs = nodes.get("nodes", {})
        if node_id not in node_configs:
            raise ValueError(f"Node '{node_id}' not found")
        return node_configs[node_id]
    
    def get_instance_order(self):
        """
        获取实例化顺序
        
        Returns:
            list: 节点 ID 列表
        """
        nodes = self.load_nodes()
        return nodes.get("instance_order", [])
    
    def get_all_nodes(self):
        """
        获取所有节点配置
        
        Returns:
            dict: 所有节点配置
        """
        nodes = self.load_nodes()
        return nodes.get("nodes", {})

