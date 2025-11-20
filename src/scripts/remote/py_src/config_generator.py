"""
配置文件生成器：执行 config_generators 命令，生成配置文件
"""
import os
import subprocess
import sys

current_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.append(current_dir)
import variable_resolver
import config_mgr


class ConfigGenerator:
    """配置文件生成器"""
    
    def __init__(self, config_manager: config_mgr.ConfigManager, 
                 variable_resolver: variable_resolver.VariableResolver,
                 config_base: str = None):
        """
        初始化配置文件生成器
        
        Args:
            config_manager: 配置管理器
            variable_resolver: 变量解析器
            config_base: 配置文件基础目录
        """
        self.config_manager = config_manager
        self.variable_resolver = variable_resolver
        
        if config_base is None:
            import util
            config_base = util.CONFIG_BASE
        
        self.config_base = config_base
        self.node_configs_dir = os.path.join(config_base, "node_configs")
        
        # 确保 node_configs 目录存在
        os.makedirs(self.node_configs_dir, exist_ok=True)
    
    def generate_node_config(self, node_id: str):
        """
        为指定节点生成配置文件
        
        Args:
            node_id: 节点 ID
        """
        node_config = self.config_manager.get_node(node_id)
        config_generators = node_config.get("config_generators", [])
        
        if not config_generators:
            print(f"No config generators defined for node {node_id}")
            return
        
        for generator in config_generators:
            command = generator.get("command", "")
            output_dir = generator.get("output_dir", f"node_configs/{node_id}")
            
            if not command:
                print(f"Warning: Empty command in config generator for node {node_id}")
                continue
            
            # 解析命令中的变量引用
            resolved_command = self.variable_resolver.resolve_command(command)
            
            # 确保输出目录存在
            full_output_dir = os.path.join(self.config_base, output_dir)
            os.makedirs(full_output_dir, exist_ok=True)
            
            # 切换到 config_base 目录执行命令
            print(f"Generating config for node {node_id}: {resolved_command}")
            print(f"Output directory: {full_output_dir}")
            
            try:
                # 在 config_base 目录下执行命令
                result = subprocess.run(
                    resolved_command,
                    shell=True,
                    cwd=self.config_base,
                    capture_output=True,
                    text=True,
                    check=True
                )
                
                if result.stdout:
                    print(f"Config generator output: {result.stdout}")
                if result.stderr:
                    print(f"Config generator warnings: {result.stderr}")
                
                print(f"Config generated successfully for node {node_id}")
                
            except subprocess.CalledProcessError as e:
                print(f"Error generating config for node {node_id}: {e.stderr}")
                raise Exception(f"Failed to generate config for node {node_id}: {e.stderr}")
    
    def get_node_config_dir(self, node_id: str) -> str:
        """
        获取节点的配置目录路径
        
        Args:
            node_id: 节点 ID
        
        Returns:
            str: 配置目录路径
        """
        node_config = self.config_manager.get_node(node_id)
        config_generators = node_config.get("config_generators", [])
        
        if config_generators:
            # 使用第一个 generator 的 output_dir
            output_dir = config_generators[0].get("output_dir", f"node_configs/{node_id}")
        else:
            output_dir = f"node_configs/{node_id}"
        
        return os.path.join(self.config_base, output_dir)

