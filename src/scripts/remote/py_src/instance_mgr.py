"""
实例化管理器：处理完整的节点实例化流程
"""
import os
import sys
import time
import tempfile
import subprocess

current_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.append(current_dir)
import config_mgr
import variable_resolver
import config_generator
import vm_mgr
import remote_device
import util
import cert_mgr
import os


class InstanceManager:
    """实例化管理器"""
    
    def __init__(self, config_base: str = None):
        """
        初始化实例化管理器
        
        Args:
            config_base: 配置文件基础目录
        """
        if config_base is None:
            config_base = util.CONFIG_BASE
        
        self.config_base = config_base
        self.config_manager = config_mgr.ConfigManager(config_base)
        self.vm_manager = vm_mgr.VMManager(backend_type="multipass")
        self.variable_resolver = variable_resolver.VariableResolver(
            self.config_manager, self.vm_manager
        )
        self.config_generator = config_generator.ConfigGenerator(
            self.config_manager, self.variable_resolver, config_base
        )
    
    def get_project_dir(self):
        """获取项目根目录"""
        return os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
    
    def create_vm_from_template(self, node_id: str, node_config: dict):
        """
        基于 VM 模板创建虚拟机
        
        Args:
            node_id: 节点 ID
            node_config: 节点配置
        """
        template_name = node_config.get("vm_template")
        if not template_name:
            raise ValueError(f"Node {node_id} does not have vm_template")
        
        vm_template = self.config_manager.get_vm_template(template_name)
        
        # 合并硬件配置
        hardware = vm_template.get("hardware", {})
        network_config = node_config.get("network", {})
        
        vm_create_config = {
            'cpu': hardware.get('cpu', 1),
            'memory': hardware.get('memory', '1G'),
            'disk': hardware.get('disk', '10G'),
            'config_base': self.config_base
        }
        
        print(f"Creating VM {node_id} from template {template_name}")
        print(f"VM config: {vm_create_config}")
        
        success = self.vm_manager.create_vm(node_id, vm_create_config)
        if not success:
            raise Exception(f"Failed to create VM {node_id}")
        
        # 等待 VM 完全启动
        time.sleep(5)
        
        # 执行 VM 模板的初始化命令
        init_commands = vm_template.get("init_commands", [])
        for cmd in init_commands:
            print(f"Executing template init command: {cmd}")
            stdout, stderr = self.vm_manager.exec_command(node_id, cmd)
            if stderr:
                print(f"Warning: Template init command failed: {stderr}")
        
        print(f"VM {node_id} created successfully")
    
    def init_node_instance(self, node_id: str, node_config: dict):
        """
        初始化节点实例
        
        Args:
            node_id: 节点 ID
            node_config: 节点配置
        """
        print(f"\nInitializing node instance: {node_id}")
        
        # 刷新节点信息（确保能获取到 IP）
        self.variable_resolver.refresh_node_info(node_id)
        
        # 执行节点初始化命令
        init_commands = node_config.get("init_commands", [])
        for cmd in init_commands:
            # 解析命令中的变量引用
            resolved_cmd = self.variable_resolver.resolve_command(cmd)
            print(f"Executing node init command: {resolved_cmd}")
            
            stdout, stderr = self.vm_manager.exec_command(node_id, resolved_cmd)
            if stderr:
                print(f"Warning: Node init command failed: {stderr}")
        
        print(f"Node {node_id} initialized successfully")
    
    def install_app(self, node_id: str, app_name: str):
        """
        安装软件到节点
        
        Args:
            node_id: 节点 ID
            app_name: 软件名称
        """
        print(f"\nInstalling app {app_name} on node {node_id}")
        
        app_config = self.config_manager.get_app(app_name)
        device = remote_device.remote_device(node_id)
        
        # 处理目录复制
        directories = app_config.get("directories", [])
        for dir_config in directories:
            source = dir_config.get("source")
            target = dir_config.get("target")
            dir_type = dir_config.get("type", "rsync")
            
            if not source or not target:
                continue
            
            project_dir = self.get_project_dir()
            source_path = os.path.join(project_dir, source)
            
            if not os.path.exists(source_path):
                print(f"Warning: Source path {source_path} does not exist, skipping...")
                continue
            
            if dir_type == "tar":
                # 创建 tar 包并上传
                print(f"Creating tar archive from {source_path}...")
                with tempfile.NamedTemporaryFile(suffix='.tar.gz', delete=False) as tmp_file:
                    tar_path = tmp_file.name
                
                subprocess.run(
                    f"cd {source_path} && tar czf {tar_path} .",
                    shell=True,
                    check=True
                )
                
                # 创建临时目录用于上传
                stdout, stderr = device.run_command("mktemp -d")
                if stderr:
                    raise Exception(f"Failed to create temp directory: {stderr}")
                remote_temp_dir = stdout.strip()
                remote_tar = os.path.join(remote_temp_dir, "app.tar.gz")
                
                # 上传 tar 包
                print(f"Uploading tar archive to {node_id}...")
                device.push(tar_path, remote_tar)
                
                # 解压
                print(f"Extracting tar archive on {node_id}...")
                device.run_command(f"mkdir -p {target}")
                device.run_command(f"cd {target} && tar xzf {remote_tar}")
                
                # 清理
                device.run_command(f"rm -rf {remote_temp_dir}")
                os.unlink(tar_path)
                
            elif dir_type == "rsync":
                # 使用 rsync 或 push 复制目录
                print(f"Copying directory {source_path} to {target} on {node_id}...")
                device.run_command(f"mkdir -p {target}")
                # 使用 push 递归复制
                device.push(source_path, target, recursive=True)
        
        # 执行安装命令
        install_commands = app_config.get("commands", {}).get("install", [])
        if isinstance(install_commands, str):
            install_commands = [install_commands]
        
        for cmd in install_commands:
            print(f"Executing install command: {cmd}")
            stdout, stderr = device.run_command(cmd)
            if stderr:
                print(f"Warning: Install command failed: {stderr}")
        
        print(f"App {app_name} installed successfully on node {node_id}")
    
    def ensure_test_ca(self):
        """
        确保测试 CA 证书存在，如果不存在则创建
        
        Returns:
            (ca_cert_path, ca_key_path) 元组
        """
        # 获取测试 CA 证书路径
        test_ca_dir = os.path.join(self.config_base, "..", "test_ca")
        os.makedirs(test_ca_dir, exist_ok=True)
        
        ca_cert_path = os.path.join(test_ca_dir, "test_ca_cert.pem")
        ca_key_path = os.path.join(test_ca_dir, "test_ca_key.pem")
        
        # 如果不存在，创建新的 CA
        if not os.path.exists(ca_cert_path) or not os.path.exists(ca_key_path):
            print("Test CA not found, creating new one...")
            cert_manager = cert_mgr.CertManager()
            cert_manager.create_ca(test_ca_dir, "BuckyOS Test CA")
        else:
            print(f"Test CA already exists: {ca_cert_path}")
        
        return ca_cert_path, ca_key_path
    
    def install_ca_certificate(self, node_id: str):
        """
        安装测试 CA 证书到节点
        
        注意：在测试环境（Linux VM）中，可以通过命令行自动安装 CA 证书。
        这不需要手工操作，完全自动化。
        
        Args:
            node_id: 节点 ID
        """
        print(f"\nInstalling test CA certificate on node {node_id}")
        
        # 确保测试 CA 存在
        ca_cert_path, _ = self.ensure_test_ca()
        
        device = remote_device.remote_device(node_id)
        
        # 检测目标系统的类型（通常是 Linux）
        # 先尝试检测 Linux 发行版
        stdout, stderr = device.run_command("cat /etc/os-release 2>/dev/null || echo 'unknown'")
        os_info = stdout.strip().lower() if stdout else ""
        
        # 确定 CA 证书目录和更新命令
        if "debian" in os_info or "ubuntu" in os_info:
            # Debian/Ubuntu
            remote_ca_dir = "/usr/local/share/ca-certificates"
            update_cmd = "update-ca-certificates"
        elif "redhat" in os_info or "centos" in os_info or "fedora" in os_info:
            # RHEL/CentOS/Fedora
            remote_ca_dir = "/etc/pki/ca-trust/source/anchors"
            update_cmd = "update-ca-trust"
        else:
            # 默认使用 Debian/Ubuntu 方法
            remote_ca_dir = "/usr/local/share/ca-certificates"
            update_cmd = "update-ca-certificates"
        
        remote_ca_cert = os.path.join(remote_ca_dir, "buckyos-test-ca.crt")
        
        # 1. 复制 CA 证书到系统 CA 证书目录
        print(f"Copying CA certificate to {node_id}...")
        device.run_command(f"mkdir -p {remote_ca_dir}")
        device.push(ca_cert_path, remote_ca_cert)
        
        # 2. 更新 CA 证书存储
        print("Updating CA certificate store...")
        stdout, stderr = device.run_command(update_cmd)
        if stderr and "error" in stderr.lower():
            print(f"Warning: Failed to update CA certificates: {stderr}")
            print("You may need to run the update command manually on the node.")
        else:
            print(f"CA certificate installed successfully on {node_id}")
            print("Note: Some applications may need to be restarted to recognize the new CA.")
    
    def apply_node_config(self, node_id: str):
        """
        应用节点配置
        
        Args:
            node_id: 节点 ID
        """
        print(f"\nApplying config for node {node_id}")
        
        # 获取配置目录
        config_dir = self.config_generator.get_node_config_dir(node_id)
        
        if not os.path.exists(config_dir):
            print(f"Warning: Config directory {config_dir} does not exist")
            return
        
        # 检查目录是否为空
        if not os.listdir(config_dir):
            print(f"Warning: Config directory {config_dir} is empty")
            return
        
        device = remote_device.remote_device(node_id)
        
        # 将配置目录复制到 VM
        # 根据节点配置确定目标目录（通常是 /opt/buckyos/etc 或其他）
        node_config = self.config_manager.get_node(node_id)
        apps = node_config.get("apps", [])
        
        # 默认配置目录
        target_config_dir = "/opt/buckyos/etc"
        
        # 如果节点有 buckyos app，使用 /opt/buckyos/etc
        # 如果有其他 app，可能需要不同的目录
        if "web3_bridge" in apps:
            target_config_dir = "/opt/web3_bridge"
        elif "buckyos" in apps:
            target_config_dir = "/opt/buckyos/etc"
        
        print(f"Copying config from {config_dir} to {target_config_dir} on {node_id}...")
        device.run_command(f"mkdir -p {target_config_dir}")
        
        # 复制配置文件
        for root, dirs, files in os.walk(config_dir):
            for file in files:
                local_file = os.path.join(root, file)
                rel_path = os.path.relpath(local_file, config_dir)
                remote_file = os.path.join(target_config_dir, rel_path)
                remote_dir = os.path.dirname(remote_file)
                
                device.run_command(f"mkdir -p {remote_dir}")
                device.push(local_file, remote_file)
        
        print(f"Config applied successfully for node {node_id}")
    
    def instance_node(self, node_id: str):
        """
        实例化单个节点（完整流程）
        
        Args:
            node_id: 节点 ID
        """
        print(f"\n{'='*60}")
        print(f"Instancing node: {node_id}")
        print(f"{'='*60}")
        
        node_config = self.config_manager.get_node(node_id)
        
        # 1. 创建 VM（如果不存在）
        if not self.vm_manager.is_vm_exists(node_id):
            self.create_vm_from_template(node_id, node_config)
        else:
            print(f"VM {node_id} already exists, skipping creation")
        
        # 2. 初始化节点实例
        self.init_node_instance(node_id, node_config)
        
        # 3. 生成配置文件
        self.config_generator.generate_node_config(node_id)
        
        # 4. 安装软件
        apps = node_config.get("apps", [])
        for app_name in apps:
            self.install_app(node_id, app_name)
        
        # 5. 应用配置
        self.apply_node_config(node_id)
        
        # 6. 安装测试 CA 证书
        self.install_ca_certificate(node_id)
        
        print(f"\nNode {node_id} instanced successfully!")
    
    def instance_all(self):
        """实例化所有节点（按 instance_order 顺序）"""
        instance_order = self.config_manager.get_instance_order()
        
        if not instance_order:
            raise ValueError("instance_order is required in nodes.json")
        
        print(f"Instance order: {instance_order}")
        
        for node_id in instance_order:
            try:
                self.instance_node(node_id)
            except Exception as e:
                print(f"Failed to instance node {node_id}: {str(e)}")
                raise  # 任何错误直接中断脚本

