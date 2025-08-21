import os
import sys
import shutil
import subprocess
import perpare_offical_installer
from pathlib import Path

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
installer_script = os.path.join(src_dir, "publish", "installer.iss")
result_root_dir = os.environ.get("BUCKYOS_BUILD_ROOT", "/opt/buckyosci")
publish_dir = os.path.join(result_root_dir, "publish")
tmp_install_dir = os.path.join(result_root_dir, "win-installer")

def make_installer(architecture, version):
    print(f"make installer with architecture: {architecture}, version: {version}")
    result_dir = os.path.join(publish_dir, version)
    if not os.path.exists(result_dir):
        os.makedirs(result_dir)

    shutil.rmtree(tmp_install_dir, ignore_errors=True)
    os.makedirs(tmp_install_dir)

    shutil.copy(installer_script, tmp_install_dir)
    print(f"copy installer script to {tmp_install_dir}")

    # dest_dir is rootfs, collection items to this NEW rootfs
    perpare_offical_installer.prepare_rootfs_for_installer(os.path.join(tmp_install_dir, "rootfs"),  "windows", architecture, version)

    print(f"run build in {tmp_install_dir}")
    iscc_arch = "x64os" if architecture == "amd64" else "arm64os"
    subprocess.run(f"iscc /DMyAppVersion={version} /DAllowArch={iscc_arch} .\\installer.iss", shell=True, check=True, cwd=tmp_install_dir)
    print(f"build installer success at {tmp_install_dir}")
    shutil.copy(f"{tmp_install_dir}/buckyos-{iscc_arch}-{version}.exe", os.path.join(result_dir, f"buckyos-windows-{architecture}-{version}.exe"))
    print(f"copy installer to {result_dir}")

if __name__ == "__main__":
    if len(sys.argv) != 3:
        print("Usage: python make_win_installer.py <architecture> <version>")
        print("  - python make_win_installer.py amd64 0.4.1+build250724")
        print("  - python make_win_installer.py aarch64 0.4.1+build250724")
        sys.exit(1)
    architecture = sys.argv[1]
    version = sys.argv[2]
    if architecture == "x86_64":
        architecture = "amd64"
    make_installer(architecture, version)