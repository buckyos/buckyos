# make macos pkg file using munkipkg
import sys
from datetime import datetime
import os
import shutil
import json
import subprocess
from pathlib import Path

import perpare_offical_installer

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
result_base_dir = "/opt/buckyosci/publish"
dest_dir = Path("/") / "opt" / "buckyosci" / "macos-pkg"
payload_dir = dest_dir / "payload"
pkg_root_dir = payload_dir / "opt" / "buckyos"

def make_pkg(architecture, version):
    print(f"make pkg with architecture: {architecture}, version: {version}")
    result_dir = os.path.join(result_base_dir, version)
    if not os.path.exists(result_dir):
        os.makedirs(result_dir)

    shutil.rmtree(dest_dir, ignore_errors=True)

    shutil.copytree(os.path.join(src_dir, "publish", "macos_pkg"), dest_dir)
    os.makedirs(pkg_root_dir)

    # prepare package payload
    # dest_dir is rootfs, collection items to this NEW rootfs
    perpare_offical_installer.prepare_rootfs_for_installer(pkg_root_dir,  "apple", architecture, version)
    
    print(f'setting pkg version: {version}')
    # modify build-info.json
    build_info = json.load(open(os.path.join(dest_dir, "build-info.json")))

    build_info["version"] = version

    json.dump(build_info, open(os.path.join(dest_dir, "build-info.json"), "w"))
    print(f"# write build-info.json to {dest_dir} OK ")

    subprocess.run(["chmod", "+x", os.path.join(dest_dir, "scripts", "postinstall")], check=True)
    subprocess.run(["chmod", "+x", os.path.join(dest_dir, "scripts", "preinstall")], check=True)

    subprocess.run(["munkipkg", dest_dir], check=True)
    print(f"# build pkg to {dest_dir} OK ")
    # copy pkg to src_dir
    pkg_file = os.path.join(dest_dir, "build", f"buckyos-{version}.pkg")
    if os.path.exists(pkg_file):
        shutil.copy(pkg_file, os.path.join(result_dir, f"buckyos-apple-{architecture}-{version}.pkg"))
        print(f"# copy pkg to {result_dir} OK ")
    else:
        print(f"# pkg file not found: {pkg_file}")
    
if __name__ == "__main__":
    if len(sys.argv) != 3:
        print("Usage: python make_mac_pkg.py <architecture> <version>")
        print("  - python make_mac_pkg.py amd64 0.4.1+build250724")
        print("  - python make_mac_pkg.py aarch64 0.4.1+build250724")
        sys.exit(1)
    architecture = sys.argv[1]
    version = sys.argv[2]
    if architecture == "x86_64":
        architecture = "amd64"
    make_pkg(architecture, version)