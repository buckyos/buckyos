import tempfile
import os
import sys
import platform
import platform

import build_web_apps
import build_rust
import prepare_rootfs
import install
import build_tray_controller

if platform.system() == "Windows":
    temp_dir = tempfile.gettempdir()
else:
    temp_dir = "/tmp/"

def build(skip_web_app, skip_install, target, with_tray_controller, auto_win_sdk):
    project_name = "buckyos"
    target_dir = os.path.join(temp_dir, "rust_build", project_name)

    if not skip_web_app:
        build_web_apps.build_web_apps()
    build_rust.build_rust(target_dir, target)
    prepare_rootfs.copy_files(os.path.join(target_dir, target))

    if with_tray_controller:
        if platform.system() == "Windows":
            build_tray_controller.prepare_win(auto_win_sdk)
        build_tray_controller.build(target)
        tray_controller_target_dir = os.path.join(temp_dir, "rust_build", "tray_controller")
        prepare_rootfs.strip_and_copy_rust_file(os.path.join(tray_controller_target_dir, target), "tray-controller", prepare_rootfs.root_bin_dir)

    if not skip_install:
        install.install()


def build_main():
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
#     elif system == "Linux" and (arch == "x86_64" or arch == "AMD64"):
#         target = "aarch64-unknown-linux-gnu"
    elif system == "Darwin" and (arch == "arm64" or arch == "arm"):
        target = "aarch64-apple-darwin"
    print(f"DEBUG: target is : {target}")

    auto_win_sdk = False
    with_tray_controller = False
    for arg in sys.argv:
        if arg == "--no-build-web-apps":
            skip_web_app = True
        if arg == "--no-install":
            skip_install = True
        if arg == "amd64":
            target = "x86_64-unknown-linux-musl"
        if arg == "aarch64":
            target = "aarch64-unknown-linux-gnu"
        if arg == "--auto-win-sdk":
            auto_win_sdk = True
        if arg == "--tray-controller":
            with_tray_controller = True
        if arg.startswith("--target="):
            target = arg.split("=")[1]


    print(f"will build buckyos: with_tray_controller={with_tray_controller}, auto_win_sdk={auto_win_sdk}")
    build(skip_web_app, skip_install, target, with_tray_controller, auto_win_sdk)
    
if __name__ == "__main__":
    build_main()