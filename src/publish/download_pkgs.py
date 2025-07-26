# 把所有平台的完整rootfs下载到发布机（有buckyos.ai的开发者私钥）

# - 基于Github Action 构建得到rootfs
# - 下载所有平台的rootfs，
# - 下载默认app的pkg,
# - 基于该完整rootfs可以构建不带自动签名的，指定平台的开发版deb(安装包)

import sys
import glob
import zipfile
import os
import tarfile

download_base_dir = "/opt/buckyosci/download"
buckyosci_rootfs_dir = "/opt/buckyosci/rootfs"
system_list = ["windows", "linux", "apple"]
machine_list = ["amd64", "aarch64"]

def unzip_rootfs(rootfs_path, target_dir):
    """
    解压rootfs压缩包
    zip中如果只有一个rootfs.tar 文件，则需要再解压到target_dir,否则直接解压到target_dir
    """
    # 确保目标目录存在
    os.makedirs(target_dir, exist_ok=True)
    
    with zipfile.ZipFile(rootfs_path, 'r') as zip_ref:
        # 获取zip文件中的所有文件列表
        file_list = zip_ref.namelist()
        
        # 检查是否只有一个rootfs.tar文件
        if len(file_list) == 1 and file_list[0].endswith('.tar'):
            # 只有一个tar文件，先解压zip，再解压tar
            zip_ref.extractall(target_dir)
            tar_file_path = os.path.join(target_dir, file_list[0])
            
            # 解压tar文件
            with tarfile.open(tar_file_path, 'r') as tar_ref:
                tar_ref.extractall(target_dir)
            print(f"unzip and tar extract done: {rootfs_path} => {target_dir}")
            # 删除临时tar文件
            os.remove(tar_file_path)
        else:
            # 直接解压zip内容到目标目录
            zip_ref.extractall(target_dir)
            print(f"unzip done: {rootfs_path} => {target_dir}")


def download_rootfs(version):
    # do download

    # unzip rootfs
    
    for os_name in system_list:
        for machine_name in machine_list:
            if os_name == "windows" and machine_name == "aarch64":
                continue
            rootfs_id = f"buckyos-{os_name}-{machine_name}"
            target_dir = os.path.join(buckyosci_rootfs_dir, version, rootfs_id)
            if not os.path.exists(target_dir):
                os.makedirs(target_dir)
            zipfile = os.path.join(download_base_dir, f"buckyos-{os_name}-{machine_name}-{version}.zip")
            unzip_rootfs(zipfile, target_dir)


    pass

if __name__ == "__main__":
    #version = "0.4.0-250724"
    version = sys.argv[1]
    download_rootfs(version)
    print("download_rootfs done")


