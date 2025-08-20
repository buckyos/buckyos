import os
import sys
import tempfile
import shutil
import subprocess
from datetime import datetime
import perpare_offical_installer
from pathlib import Path

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
rootfs_dir = os.path.join(src_dir, "rootfs")
installer_script = os.path.join(src_dir, "publish", "installer.iss")
result_base_dir = "/opt/buckyosci/publish"
dest_dir = Path("/") / "opt" / "buckyosci" / "windows-installer"

def make_installer(architecture, version):
    print(f"make deb with architecture: {architecture}, version: {version}")
    result_dir = os.path.join(result_base_dir, version)
    if not os.path.exists(result_dir):
        os.makedirs(result_dir)

    if not os.path.exists(dest_dir):
        os.makedirs(dest_dir)

    shutil.copy(installer_script, dest_dir)
    print(f"copy installer script to {dest_dir}")

    # dest_dir is rootfs, collection items to this NEW rootfs
    perpare_offical_installer.prepare_rootfs_for_installer(dest_dir / "rootfs",  "windows", architecture, version)

    print(f"run build in {dest_dir}")
    subprocess.run(f"iscc /DMyAppVersion={version} /DAllowArch=x64os .\\installer.iss", shell=True, check=True, cwd=dest_dir)
    print(f"build installer success at {dest_dir}")
    shutil.copy(f"{dest_dir}/buckyos-x64os-{version}.exe", result_base_dir)
    print(f"copy installer to {result_base_dir}")

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