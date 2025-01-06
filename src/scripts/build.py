import tempfile
import os
import sys
import platform

import build_web_apps
import build_rust
import perpare_rootfs
import install

def build(skip_web_app, skip_install, target):
    temp_dir = tempfile.gettempdir()
    project_name = "buckyos"
    target_dir = os.path.join(temp_dir, "rust_build", project_name)

    if not skip_web_app:
        build_web_apps.build_web_apps()
    build_rust.build_rust(target_dir, target)
    perpare_rootfs.copy_files(os.path.join(target_dir, target))
    if not skip_install:
        install.install()

if __name__ == "__main__":
    skip_web_app = False
    skip_install = False
    system = platform.system() # Linux / Windows / Darwin
    arch = platform.machine() # x86_64 / AMD64 / arm64 / arm
    print(f"DEBUG: system:{system},arch:{arch}")
    target = ""
    if system == "Linux" and (arch == "x86_64" or arch == "AMD64"):
        target = "x86_64-unknown-linux-musl"
    elif system == "Windows" and (arch == "x86_64" or arch == "AMD64"):
        target = "x86_64-pc-windows-msvc"
    elif system == "Linux" and (arch == "x86_64" or arch == "AMD64"):
        target = "aarch64-unknown-linux-gnu"
    elif system == "Darwin" and (arch == "arm64" or arch == "arm"):
        target = "aarch64-apple-darwin"
    print(f"DEBUG: target is : {target}")

    for arg in sys.argv:
        if arg == "--no-build-web-apps":
            skip_web_app = True
        if arg == "--no-install":
            skip_install = True
        if arg == "amd64":
            target = "x86_64-unknown-linux-musl"
        if arg == "aarch64":
            target = "aarch64-unknown-linux-gnu"
    build(skip_web_app, skip_install, target)
