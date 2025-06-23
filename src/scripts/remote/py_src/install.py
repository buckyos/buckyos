import sys
import os
# import json
import tempfile
import subprocess
import remote_device
import get_device_info
import util


def get_project_dir():
    project_root =  os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
    return project_root

def create_rootfs_tarball():
    """创建rootfs的tar包"""
    # 获取当前工程根目录
    project_root =  get_project_dir()
    rootfs_path = os.path.join(project_root, "rootfs")
    
    print(f"rootfs_path: {rootfs_path}")
    if not os.path.exists(rootfs_path):
        print("rootfs directory not found")
        sys.exit(1)

    # 检查是否存在bin文件
    node_daemon_bin_path = os.path.join(rootfs_path, "bin", "node_daemon", "node_daemon")
    if not os.path.exists(node_daemon_bin_path):
        print(f"没有编译， node_daemon_bin_path : {node_daemon_bin_path}")
        sys.exit(1)

    
    # 创建临时tar包
    with tempfile.NamedTemporaryFile(suffix='.tar.gz', delete=False) as tmp_file:
        tar_path = tmp_file.name
    
    # 打包rootfs目录
    subprocess.run(
        f"cd {rootfs_path} && tar czf {tar_path} .",
        shell=True,
        check=True
    )

    return tar_path


def install_sn(device): 
        # sn 节点
    print("uploading web3_bridge ...")
    project_dir = get_project_dir()
    print(f"project_dir, {project_dir}")
    device.run_command("sudo mkdir -p /opt/web3_bridge")
    device.scp_put(f"{project_dir}/web3_bridge/start.py", "/opt/web3_bridge/start.py")
    device.scp_put(f"{project_dir}/web3_bridge/stop.py", "/opt/web3_bridge/stop.py")
    device.scp_put(f"{project_dir}/web3_bridge/web3_gateway", "/opt/web3_bridge/web3_gateway")
    print("web3_bridge uploaded")



def install(device_id: str):
    device = remote_device.remote_device(device_id)
    try:
        if  device.has_app("web3_bridge"):
            install_sn(device)
            sys.exit(0)


        # 1. 创建tar包
        print("Creating rootfs tarball...")
        tar_path = create_rootfs_tarball()
        print(f"tar_path: {tar_path}")

        # 2. 检查远程目录是否存在
        stdout, stderr = device.run_command("test -d /opt/buckyos && echo 'exists' || echo 'not_exists'")
        is_fresh_install = 'not_exists' in stdout
        
        # 3. 创建临时目录用于上传
        stdout, stderr = device.run_command("mktemp -d")
        if stderr:
            raise Exception(f"Failed to create temp directory: {stderr}")
        remote_temp_dir = stdout.strip()
        
        # 4. 上传tar包
        print("Uploading rootfs...")
        remote_tar = os.path.join(remote_temp_dir, "rootfs.tar.gz")
        device.scp_put(tar_path, remote_tar)
        
        
        # 5. 安装过程
        if is_fresh_install:
            print("Performing fresh installation...")
            install_commands = [
                "mkdir -p /opt/buckyos",
                f"cd /opt/buckyos && tar xzf {remote_tar}",
                "mkdir -p /opt/buckyos/etc"
            ]
        else:
            print("Updating existing installation...")
            install_commands = [
                "rm -rf /opt/buckyos/bin",
                f"cd /opt/buckyos && tar xzf {remote_tar} ./bin",
            ]

        for cmd in install_commands:
            print(f"Running remote command: {cmd}")
            stdout, stderr = device.run_command(cmd)
            if stderr:
                raise Exception(f"Installation failed: {stderr}")
        
        # 6. 如果是新安装，复制配置文件
        #if is_fresh_install and 'identity_file' in device.config:
        #    local_identity = device.config['identity_file']
        #     if os.path.exists(local_identity):
        #        remote_identity = "/opt/buckyos/etc/device.conf"
        #        scp_command = f"scp {local_identity} {device.config['username']}@{device.config['hostname']}:{remote_identity}"
        #        subprocess.run(scp_command, shell=True, check=True)
        
        # 7. 清理临时文件
        device.run_command(f"rm -rf {remote_temp_dir}")
        os.unlink(tar_path)
        
        print("Installation completed successfully!")
        if is_fresh_install:
            print("Performed fresh installation to /opt/buckyos/")
        else:
            print("Updated /opt/buckyos/bin/ directory")
        
        return True
        
    except Exception as e:
        print(f"Error during installation: {str(e)}", file=sys.stderr)
        return False


def main():
    if len(sys.argv) < 3:
        print("Usage: install.py <device_id>")
        print("Usage: install.py --all")
        return
    device_id = sys.argv[2]
    if device_id == "--all":
        all_devices = get_device_info.read_from_config(info_path=util.VM_DEVICE_CONFIG)
        for device_id in all_devices:
            print(f"install target device_id: {device_id}")
            install(device_id)
    else:
        print(f"install target device_id: {device_id}")
        install(device_id)