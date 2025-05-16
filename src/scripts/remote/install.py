#!/usr/bin/env python3
# install.py $device_id 

# 将工程目录下准备好的 rootfs目录先打好一个tar包上传,然后在进行安装.
# 安装的时候如果目标设备的 /opt/buckyos目录不存在,则会把tar包的内容释放到 /opt/buckyos,
# 把保存在配置文件中的device_id的身份配置文件复制到/opt/buckyos/etc目录,
# 如果/opt/buckyos目录存在,则只更新/opt/buckyos/bin 目录

import sys
import os
import json
import tempfile
import subprocess
from remote_device import remote_device

def print_usage():
    print("Usage: install.py <device_id>")
    sys.exit(1)

def get_project_dir():
    project_root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    return project_root

def create_rootfs_tarball():
    """创建rootfs的tar包"""
    # 获取当前工程根目录
    project_root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    rootfs_path = os.path.join(project_root, "rootfs")
    
    print(f"rootfs_path: {rootfs_path}")
    if not os.path.exists(rootfs_path):
        raise Exception("rootfs directory not found")
    
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



def install(device_id: str):
    device = remote_device(device_id)
    
    try:
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

        if device.has_app("web3_bridge"):
            print("uploading web3_bridge ...")
            project_dir = get_project_dir()
            device.scp_put(f"{project_dir}/web3_bridge", "/opt/web3_bridge", recursive=True)

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
