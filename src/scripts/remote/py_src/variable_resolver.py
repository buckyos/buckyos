"""
变量解析器：解析配置文件中的变量引用，通过 vm_mgr 获取实时状态
"""
import re
import sys
import os

current_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.append(current_dir)
import vm_mgr
import config_mgr


class VariableResolver:
    """变量解析器"""
    
    # 支持的变量格式：{{node_id.ip}}, {{node_id.zone_id}}, {{node_id.node_id}}
    VARIABLE_PATTERN = re.compile(r'\{\{(\w+)\.(\w+)\}\}')
    
    def __init__(self, config_manager: config_mgr.ConfigManager, vm_manager: vm_mgr.VMManager):
        """
        初始化变量解析器
        
        Args:
            config_manager: 配置管理器
            vm_manager: VM 管理器
        """
        self.config_manager = config_manager
        self.vm_manager = vm_manager
        self._node_cache = {}  # 缓存已解析的节点信息
    
    def get_node_info(self, node_id: str) -> dict:
        """
        获取节点的实时信息
        
        Args:
            node_id: 节点 ID
        
        Returns:
            dict: 节点信息，包含 ip, zone_id, node_id 等
        """
        if node_id in self._node_cache:
            return self._node_cache[node_id]
        
        info = {}
        
        # 从 nodes.json 获取静态信息
        try:
            node_config = self.config_manager.get_node(node_id)
            info['zone_id'] = node_config.get('zone_id', '')
            info['node_id'] = node_config.get('node_id', node_id)
        except ValueError:
            # 节点不存在于配置中，可能还未创建
            info['zone_id'] = ''
            info['node_id'] = node_id
        
        # 从 vm_mgr 获取实时信息（IP 地址等）
        try:
            if self.vm_manager.is_vm_exists(node_id):
                vm_ips = self.vm_manager.get_vm_ip(node_id)
                if vm_ips and len(vm_ips) > 0:
                    info['ip'] = vm_ips[0] if isinstance(vm_ips, list) else vm_ips
                else:
                    info['ip'] = ''
            else:
                info['ip'] = ''
        except Exception as e:
            print(f"Warning: Failed to get VM info for {node_id}: {e}")
            info['ip'] = ''
        
        self._node_cache[node_id] = info
        return info
    
    def resolve_variable(self, var_name: str, var_attr: str) -> str:
        """
        解析单个变量
        
        Args:
            var_name: 变量名（节点 ID）
            var_attr: 变量属性（ip, zone_id, node_id）
        
        Returns:
            str: 解析后的值
        """
        node_info = self.get_node_info(var_name)
        
        if var_attr == 'ip':
            return node_info.get('ip', '')
        elif var_attr == 'zone_id':
            return node_info.get('zone_id', '')
        elif var_attr == 'node_id':
            return node_info.get('node_id', '')
        else:
            raise ValueError(f"Unsupported variable attribute: {var_attr}")
    
    def resolve_string(self, text: str) -> str:
        """
        解析字符串中的所有变量引用
        
        Args:
            text: 包含变量引用的字符串
        
        Returns:
            str: 解析后的字符串
        """
        def replace_var(match):
            node_id = match.group(1)
            attr = match.group(2)
            value = self.resolve_variable(node_id, attr)
            return value
        
        return self.VARIABLE_PATTERN.sub(replace_var, text)
    
    def resolve_command(self, command: str) -> str:
        """
        解析命令中的变量引用
        
        Args:
            command: 包含变量引用的命令
        
        Returns:
            str: 解析后的命令
        """
        return self.resolve_string(command)
    
    def clear_cache(self):
        """清除缓存"""
        self._node_cache.clear()
    
    def refresh_node_info(self, node_id: str):
        """刷新指定节点的信息"""
        if node_id in self._node_cache:
            del self._node_cache[node_id]
        return self.get_node_info(node_id)

