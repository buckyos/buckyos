# 准备package 的待pack目录
# 删除 /tmp/buckyos_pkgs/目录
# 先复制../publish/buckyos_pkgs/$pkg_name 到 /tmp/buckyos_pkgs/$pkg_name 目录
# $pkg_name 是包名，包含系统perfix(比如 nightly-apple-x86_64.)
# 将 ../rootfs/bin/$app_name 下面的目录复制到 /tmp/buckyos_pkgs/$pkg_name 
import os
import shutil
import glob
import tempfile
import platform
import time
import json

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
channel = "nightly"
platform_name = platform.system().lower()
if platform_name == "darwin":
    platform_name = "apple"
machine_name = platform.machine()
if machine_name == "arm64":
    machine_name = "aarch64"

perfix = channel + "-" + platform_name + "-" + machine_name

if platform_name == "windows":
    sys_temp_dir = tempfile.gettempdir()
else:
    sys_temp_dir = "/tmp/"

def prepare_package(pkg_name):
    tmp_dir = os.path.join(sys_temp_dir, "buckyos_pkgs")
    if not os.path.exists(tmp_dir):
        os.makedirs(tmp_dir)
    real_pkg_name = perfix + "." + pkg_name
    pkg_tmp_dir = os.path.join(tmp_dir, real_pkg_name)
    if os.path.exists(pkg_tmp_dir):
        shutil.rmtree(pkg_tmp_dir)

    os.makedirs(pkg_tmp_dir, exist_ok=True)
    
    # 复制../publish/buckyos_pkgs/$pkg_name 到 /tmp/buckyos_pkgs/$pkg_name 目录
    src_pkg_dir = os.path.join(src_dir,"publish/buckyos_pkgs", pkg_name)
    if os.path.exists(src_pkg_dir):
        print(f"Copying {src_pkg_dir} to {pkg_tmp_dir}")
        for item in os.listdir(src_pkg_dir):
            s = os.path.join(src_pkg_dir, item)
            d = os.path.join(pkg_tmp_dir, item)
            if os.path.isdir(s):
                shutil.copytree(s, d)
            else:
                shutil.copy2(s, d)
    
    # 将 ../rootfs/bin/$app_name 下面的目录复制到 /tmp/buckyos_pkgs/$pkg_name
    app_name = pkg_name
    app_dir = os.path.join(src_dir,"rootfs/bin", app_name)
    if os.path.exists(app_dir):
        print(f"Copying {app_dir} to {pkg_tmp_dir}")
        for item in os.listdir(app_dir):
            s = os.path.join(app_dir, item)
            d = os.path.join(pkg_tmp_dir, item)
            if os.path.isdir(s):
                shutil.copytree(s, d)
            else:
                shutil.copy2(s, d)
    
    print(f"Package {pkg_name} prepared at {pkg_tmp_dir}")

def main():
    # 获取所有包名
    pkg_dirs = glob.glob(os.path.join(src_dir, "publish/buckyos_pkgs/*"))
    for pkg_dir in pkg_dirs:
        pkg_name = os.path.basename(pkg_dir)
        meta_file = os.path.join(pkg_dir, ".pkg_meta.json")
        if os.path.exists(meta_file):
            pkg_meta = json.load(open(meta_file))
            pkg_meta["pub_time"] = int(time.time())
            pkg_name = pkg_meta["pkg_name"]
            pkg_meta["pkg_name"] = perfix + "." + pkg_name 
            with open(meta_file, "w") as f:
                json.dump(pkg_meta, f)
            prepare_package(pkg_name)

if __name__ == "__main__":
    main()
