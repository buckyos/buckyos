## pack_pkgs.py $version
#
# 这一步需要有buckyos.ai的开发者私钥 
# - 挨个读取/opt/buckyos_pkgs/$version/下的目录，并调用buckycli的pack_pkg命令
# - pack_pkg的结果，放到 /opt/buckyos_pack_pkgs/$version/目录下
import sys
import os
import glob
import subprocess
import platform
import json
import shutil
import time
from urllib.request import urlretrieve

publish_root_dir = os.path.dirname(os.path.abspath(__file__))
system_list = ["windows", "linux", "apple"]
machine_list = ["amd64", "aarch64"]
rootfs_base_dir = "/opt/buckyosci/rootfs/"
target_base_dir = "/opt/buckyosci/pack_pkgs/"
app_base_dir = "/opt/buckyosci/apps/"
app_raw_dir = "/opt/buckyosci/app_build/"
base_meta_db_url = "https://buckyos.ai/ndn/repo/meta_index.db/content"

buckycli_path = os.getenv("BUCKYCLI_PATH", "/opt/buckyos/bin/buckycli/buckycli")


def pack_packages(pkg_dir, target_dir):
    """打包所有有效的包"""
    packed_dirs = []
    
    # 扫描所有包目录
    pkg_dirs = glob.glob(os.path.join(pkg_dir, "*"))
    for pkg_path in pkg_dirs:
        if not os.path.isdir(pkg_path):
            continue
        print(f"# pack {pkg_path}")
            
        # 检查是否有有效的pkg_meta.json
        meta_file = os.path.join(pkg_path, "pkg_meta.json")
        if not os.path.exists(meta_file):
            print(f"跳过 {pkg_path}: 没有找到 pkg_meta.json")
            continue
            
        try:
            with open(meta_file, 'r') as f:
                meta_data = json.load(f)
                if "pkg_name" not in meta_data or "version" not in meta_data:
                    print(f"跳过 {pkg_path}: pkg_meta.json 缺少必要字段")
                    continue
                    
            # 调用buckycli pack_pkg命令打包
            
            cmd = [buckycli_path, "pack_pkg", pkg_path, target_dir]
            print(f"执行命令: {' '.join(cmd)}")
            result = subprocess.run(cmd, capture_output=True, text=True)
            
            if result.returncode == 0:
                pkg_name = os.path.basename(pkg_path)
                packed_dir = os.path.join(target_dir, pkg_name)
                packed_dirs.append(packed_dir)
                print(f"成功打包 {pkg_name}")
            else:
                print(f"打包 {pkg_path} 失败: {result.stderr}")
                
        except Exception as e:
            print(f"处理 {pkg_path} 时出错: {str(e)}")
    
    return packed_dirs


def copy_rootfs(src_dir:str, target_dir:str):
    if os.path.exists(target_dir):
        shutil.rmtree(target_dir)
    os.makedirs(target_dir)
    #复制src_dir目录下除bin目录外的所有item到target_dir
    for item in os.listdir(src_dir):
        if item == "bin":
            os.makedirs(os.path.join(target_dir, item))
            for sub_item in os.listdir(os.path.join(src_dir, item)):
                src_sub_item_path = os.path.join(src_dir, item, sub_item)
                target_sub_item_path = os.path.join(target_dir, item, sub_item)
                
                if not os.path.isdir(src_sub_item_path):
                    print(f"copy file {src_sub_item_path} => {target_sub_item_path}")
                    shutil.copy(src_sub_item_path, target_sub_item_path)
            
            continue

        src_item_path = os.path.join(src_dir, item)
        target_item_path = os.path.join(target_dir, item)
        if os.path.isdir(src_item_path):
            shutil.copytree(src_item_path, target_item_path)
        else:
            shutil.copy(src_item_path, target_item_path)
    print(f"copy {src_dir} => {target_dir}")
    


# 这里no_copy_app是给prepare installer准备的，因为installer的bin下已经有正确的app文件了。不需要再拷贝一次
def prepare_package(src_pkg_dir, prefix, version):
    # 使用pkg dir的子文件夹名作为pkg_id
    pkg_id = os.path.basename(src_pkg_dir)
    meta_file = os.path.join(publish_root_dir, "buckyos_pkgs", pkg_id, "pkg_meta.json")
    if not os.path.exists(meta_file):
        return
    
    pkg_meta = json.load(open(meta_file))
    pkg_meta["pub_time"] = int(time.time())
    pkg_meta["exp"] = int(time.time()) + 3600 * 24 * 365 * 3
    #pkg_name = pkg_meta["pkg_name"]
    pkg_meta["pkg_name"] = prefix + "." + pkg_id
    pkg_meta["version"] = version
    deps = pkg_meta.get("deps",{})
    new_deps = {}
    for dep_pkg_name,dep_pkg_version in deps.items():
        new_deps[prefix + "." + dep_pkg_name] = dep_pkg_version
    pkg_meta["deps"] = new_deps

  
    pkg_meta_file = os.path.join(src_pkg_dir,"pkg_meta.json") 
    json.dump(pkg_meta, open(pkg_meta_file, "w"))

    print(f"> Package {pkg_id} prepared at {src_pkg_dir}")


def pack_rootfs_pkgs(rootfs_version: str):
    target_dir = os.path.join(target_base_dir, rootfs_version)
    shutil.rmtree(target_dir, ignore_errors=True)
    if not os.path.exists(target_dir):
        os.makedirs(target_dir)
    
    channel_name = "nightly"

    for sys_name in system_list:
        for machine_name in machine_list:
            if sys_name == "windows" and machine_name == "aarch64":
                print(f"skip {sys_name}-{machine_name}")
                continue
            print(f"pack {sys_name}-{machine_name}")

            rootfs_id = f"buckyos-{sys_name}-{machine_name}"
            rootfs_dir = os.path.join(rootfs_base_dir, rootfs_version,rootfs_id)
            rootfs_bin_dir = os.path.join(rootfs_dir,"bin")

            print(f"prepare pkgs in {rootfs_bin_dir}")
            prefix = f"{channel_name}-{sys_name}-{machine_name}"
            pkg_dirs = glob.glob(os.path.join(rootfs_bin_dir,"*"))
            for pkg_dir in pkg_dirs:
                print(f"pkg_dir: {pkg_dir} ...")
                prepare_package(pkg_dir, prefix, rootfs_version)
            
            print(f"pack pkgs in {rootfs_bin_dir}")
            pack_packages(rootfs_bin_dir, target_dir)
            target_rootfs_dir = os.path.join(target_dir, rootfs_id)
            copy_rootfs(rootfs_dir, target_rootfs_dir)
    
    print(f"pack rootfs pkgs to {target_dir} done")

def prepare_meta_db(rootfs_version:str):
    target_dir = os.path.join(target_base_dir, rootfs_version)
    meta_db_path = os.path.join(target_dir, "meta_index.db")
    urlretrieve(base_meta_db_url, meta_db_path)
    # subprocess.run(["wget",base_meta_db_url,"-O",root_env_db_path], check=True)
    print(f"# download {base_meta_db_url} => {meta_db_path}")

    # 2 scan packed pkgs dir, add pkg_meta_info to meta db
    pkg_items = glob.glob(os.path.join(target_dir, "*"))
    for pkg_item in pkg_items:
        if os.path.isdir(pkg_item):
            pkg_item = os.path.join(pkg_item, "pkg_meta.json")
            if os.path.exists(pkg_item):
                print(f"# add pkg_meta_info to meta db from {pkg_item}")
                subprocess.run([buckycli_path,"set_pkg_meta",pkg_item,meta_db_path], check=True)
                print(f"# add pkg_meta_info to meta db from {pkg_item} OK")
    print(f"# update meat_index_db success, => {meta_db_path}")

    fileobj_path = os.path.join(target_dir,"buckyos-linux-amd64/local/node_daemon/root_pkg_env/pkgs/meta_index.db.fileobj")
    fileobj = json.load(open(fileobj_path))
    current_time = int(time.time())
    fileobj["create_time"] = current_time
    fileobj_target_path = os.path.join(target_dir,"meta_index.db.fileobj")
    json.dump(fileobj, open(fileobj_target_path, "w"))

    print(f"# update fileobj ({fileobj_path}) create_time to {current_time} => {fileobj_target_path}")


def add_app_pkgs(rootfs_version:str,app_name:str,app_version:str):
    target_dir = os.path.join(target_base_dir, rootfs_version)
    app_dir = os.path.join(app_base_dir, app_name,app_version)
    
    if not os.path.exists(app_dir):
        print(f"# app {app_name} {app_version} not found in {app_base_dir}")
        return
    else: 
        print(f"# add app {app_name} {app_version} pkgs from {app_dir} ...")

    packed_pkgs = glob.glob(os.path.join(app_dir,"*"))
    for packed_pkg in packed_pkgs:
        if os.path.isdir(packed_pkg):
            packed_pkg_meta_file = os.path.join(packed_pkg,"pkg_meta.json")
            if os.path.exists(packed_pkg_meta_file):
                shutil.copytree(packed_pkg, os.path.join(target_dir, os.path.basename(packed_pkg)),dirs_exist_ok=True)
                print(f"# copy packed_app_pkg {packed_pkg} => {target_dir} OK")

if __name__ == "__main__":
    root_version = sys.argv[1]
    pack_rootfs_pkgs(root_version)

    # app的包已经打好了，放在 /opt/buckyosci/apps/buckyos-filebrowser/0.4.1/目录下
    # 如果有多个default app,则调用多行add_app_pkgs
    # TODO 这里版本号是写死的，应该有更完整的配置文件，描述一个版本里所携带的app列表
    add_app_pkgs(root_version,"buckyos-filebrowser","0.4.1")
    prepare_meta_db(root_version)