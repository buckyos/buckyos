# 准备package 的待pack目录
# 删除 /tmp/buckyos_pkgs/目录
# 先复制../publish/buckyos_pkgs/$pkg_name 到 /tmp/buckyos_pkgs/$pkg_name 目录
# $pkg_name 是包名，包含系统perfix(比如 nightly-apple-x86_64.)
# 将 ../rootfs/bin/$app_name 下面的目录复制到 /tmp/buckyos_pkgs/$pkg_name 
import os
import platform

import prepare_packages
from datetime import datetime

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
channel = "nightly"
platform_name = platform.system().lower()
machine_name = platform.machine()

print(f"machine_name: {machine_name}")

if __name__ == "__main__":
    date = datetime.now().strftime("%Y%m%d")
    prepare_packages.perpare_all(channel, platform_name, machine_name, "0.4.0", date)
