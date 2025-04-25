
import sys
import os
import glob
import json
import time
import tempfile
import shutil

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
tmp_dir = os.path.join(tempfile.gettempdir(), "buckyos_pkgs")

def prepare_package(src_pkg_dir, pkg_name, pkg_meta):
    real_pkg_name = pkg_meta["pkg_name"]
    pkg_tmp_dir = os.path.join(tmp_dir, real_pkg_name)
    os.makedirs(pkg_tmp_dir)
    
    # 复制../publish/buckyos_pkgs/$pkg_name 到 /tmp/buckyos_pkgs/$pkg_name 目录

    if os.path.exists(src_pkg_dir):
        print(f"Copying {src_pkg_dir} to {pkg_tmp_dir}")
        shutil.copytree(src_pkg_dir, pkg_tmp_dir, dirs_exist_ok=True)
            
    pkg_meta_file = os.path.join(pkg_tmp_dir,"pkg_meta.json") 
    json.dump(pkg_meta, open(pkg_meta_file, "w"))

    # 将 ../rootfs/bin/$app_name 下面的目录复制到 /tmp/buckyos_pkgs/$pkg_name
    app_dir = os.path.join(src_dir,"rootfs/bin", pkg_name)
    if os.path.exists(app_dir):
        print(f"Copying {app_dir} to {pkg_tmp_dir}")
        shutil.copytree(app_dir, pkg_tmp_dir, dirs_exist_ok=True)

        print(f"Copying scripts to {pkg_tmp_dir}/scripts")
        os.makedirs(os.path.join(pkg_tmp_dir, "scripts"), exist_ok=True)
        scripts_dir = os.path.join(src_dir,"rootfs/bin/scripts")
        shutil.copytree(scripts_dir, os.path.join(pkg_tmp_dir, "scripts"), ignore=lambda src,names: ['pkg_meta.json', '.*'], dirs_exist_ok=True)
    
    print(f"> Package {pkg_name} prepared at {pkg_tmp_dir}")

def perpare_all(channel, target_os, target_arch, version, builddate):
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

    print(f'prepare packages for {prefix}')
    # Nightly-windows-amd64.node_daemon#0.4.0-build250414

    if os.path.exists(tmp_dir):
        shutil.rmtree(tmp_dir)
        os.makedirs(tmp_dir)
    # 获取所有包名
    pkg_dirs = glob.glob(os.path.join(src_dir, "publish/buckyos_pkgs/*"))
    for pkg_dir in pkg_dirs:
        if not os.path.isdir(pkg_dir):
            continue
        print(f"\n# Processing {pkg_dir}\n")
        pkg_name = os.path.basename(pkg_dir)
        meta_file = os.path.join(pkg_dir, "pkg_meta.json")
        if os.path.exists(meta_file):
            pkg_meta = json.load(open(meta_file))
            pkg_meta["pub_time"] = int(time.time())
            pkg_meta["exp"] = int(time.time()) + 3600 * 24 * 365 * 3
            #pkg_name = pkg_meta["pkg_name"]
            pkg_meta["pkg_name"] = prefix + "." + pkg_name 
            deps = pkg_meta.get("deps",{})
            new_deps = {}
            for dep_pkg_name,dep_pkg_version in deps.items():
                new_deps[prefix + "." + dep_pkg_name] = dep_pkg_version
            pkg_meta["deps"] = new_deps
            prepare_package(pkg_dir, pkg_name, pkg_meta)
    pass

def help():
    print("Usage: python prepare_packages.py <channel> <os> <arch> <version> <builddate>")

if __name__ == "__main__":
    # Usage: python scripts/prepare_packages.py <channel> <os> <arch> <version> <builddate>
    if len(sys.argv) != 6:
        help()
        exit(1)

    perpare_all(sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5])