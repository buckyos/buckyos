
import sys
import os
import glob
import json
import time
import tempfile
import shutil

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
tmp_dir = os.path.join(tempfile.gettempdir(), "buckyos_pkgs")
if os.path.exists(tmp_dir):
    shutil.rmtree(tmp_dir)
    os.makedirs(tmp_dir)

# 这里no_copy_app是给prepare installer准备的，因为installer的bin下已经有正确的app文件了。不需要再拷贝一次
def prepare_package(src_pkg_dir, prefix, version, target_dir, no_copy_app=False):
    # 使用pkg dir的子文件夹名作为pkg_id
    pkg_id = os.path.basename(src_pkg_dir)
    meta_file = os.path.join(src_pkg_dir, "pkg_meta.json")
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

    pkg_target_dir = os.path.join(target_dir, pkg_id)
    os.makedirs(pkg_target_dir, exist_ok=True)
    
    # 复制../publish/buckyos_pkgs/$pkg_name 到 /tmp/buckyos_pkgs/$pkg_name 目录

    print(f"Copying {src_pkg_dir} to {pkg_target_dir}")
    shutil.copytree(src_pkg_dir, pkg_target_dir, dirs_exist_ok=True)
        
            
    pkg_meta_file = os.path.join(pkg_target_dir,"pkg_meta.json") 
    json.dump(pkg_meta, open(pkg_meta_file, "w"))

    if not no_copy_app:
        # 将 ../rootfs/bin/$pkg_id 下面的目录复制到 /tmp/buckyos_pkgs/$pkg_name
        app_dir = os.path.join(src_dir,"rootfs/bin", pkg_id)
        if os.path.exists(app_dir):
            print(f"Copying {app_dir} to {pkg_target_dir}")
            shutil.copytree(app_dir, pkg_target_dir, dirs_exist_ok=True)

            print(f"Copying scripts to {pkg_target_dir}/scripts")
            os.makedirs(os.path.join(pkg_target_dir, "scripts"), exist_ok=True)
            scripts_dir = os.path.join(src_dir,"rootfs/bin/scripts")
            shutil.copytree(scripts_dir, os.path.join(pkg_target_dir, "scripts"), ignore=lambda src,names: ['pkg_meta.json', '.*'], dirs_exist_ok=True)
    
    print(f"> Package {pkg_id} prepared at {pkg_target_dir}")

def perpare_all(channel, target_os, target_arch, version, builddate, target_dir=tmp_dir, no_copy_app=False):
    # 规范os字符串
    if target_os == "darwin":
        target_os = "apple"
    # 规范arch字符串
    if target_arch == "arm64":
        target_arch = "aarch64"
    if target_arch == "x86_64":
        target_arch = "amd64"
    # 这里必须要从外部输入，因为host和target不一定一致。要打包的二进制可能不能执行
    prefix = f'{channel}-{target_os}-{target_arch}'
    # builddate取后6位，20250428 -> 250428
    if len(builddate) > 6:
        builddate = builddate[-6:]
    version = f'{version}-build{builddate}'

    print(f'prepare packages for {prefix}')
    # Nightly-windows-amd64.node_daemon#0.4.0-build250414
    # 获取所有包名
    pkg_dirs = glob.glob(os.path.join(src_dir, "publish/buckyos_pkgs/*"))
    for pkg_dir in pkg_dirs:
        if not os.path.isdir(pkg_dir):
            continue
        print(f"\n# Processing {pkg_dir}\n")
        prepare_package(pkg_dir, prefix, version, target_dir, no_copy_app)
    pass

def help():
    print("Usage: python prepare_packages.py <channel> <os> <arch> <version> <builddate>")

if __name__ == "__main__":
    # Usage: python scripts/prepare_packages.py <channel> <os> <arch> <version> <builddate>
    if len(sys.argv) != 6:
        help()
        exit(1)

    perpare_all(sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5])