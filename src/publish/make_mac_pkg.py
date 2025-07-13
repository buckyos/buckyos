# make macos pkg file using munkipkg
import sys
from datetime import datetime
import os
import shutil
import json
import subprocess

import perpare_installer

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
rootfs_dir = os.path.join(src_dir, "rootfs")
pkg_dir = os.path.join(src_dir, "publish", "macos_pkg")
payload_dir = os.path.join(pkg_dir, "payload")
dest_dir = os.path.join(payload_dir, "opt", "buckyos")


def make_pkg(channel, version, builddate, noBuild):
    shutil.rmtree(payload_dir, ignore_errors=True)

    # prepare package payload
    perpare_installer.prepare_installer(dest_dir, channel, "macos", "amd64", version, builddate)
    
    print(f'setting pkg version: {version}-{builddate}')
    # modify build-info.json
    build_info = json.load(open(os.path.join(pkg_dir, "build-info.json")))

    build_info["version"] = f'{version}-{builddate}'

    json.dump(build_info, open(os.path.join(pkg_dir, "build-info.json"), "w"))
    print(f"# write build-info.json to {pkg_dir} OK ")

    subprocess.run(["chmod", "+x", os.path.join(pkg_dir, "scripts", "postinstall")], check=True)
    subprocess.run(["chmod", "+x", os.path.join(pkg_dir, "scripts", "preinstall")], check=True)

    if not noBuild:
        subprocess.run(["munkipkg", pkg_dir], check=True)
        print(f"# build pkg to {pkg_dir} OK ")
        # copy pkg to src_dir
        pkg_file = os.path.join(pkg_dir, "build", f"buckyos-{version}-{builddate}.pkg")
        if os.path.exists(pkg_file):
            shutil.copy(pkg_file, os.path.join(src_dir, f"buckyos-{channel}-{version}-{builddate}.pkg"))
            print(f"# copy pkg to {src_dir} OK ")
        else:
            print(f"# pkg file not found: {pkg_file}")

if __name__ == "__main__":
    print("make sure YOU already run build.py!!!")

    version = "0.4.0"
    builddate = datetime.now().strftime("%Y%m%d")
    channel = "nightly"
    noBuild = False

    for arg in sys.argv[1:]:
        if arg == "--no-build":
            noBuild = True
        elif arg.startswith("--builddate="):
            date = arg.split("=")[1]
        elif arg.startswith("--version="):
            version = arg.split("=")[1]
        elif arg.startswith("--channel="):
            channel = arg.split("=")[1]
        else:
            version = arg

    make_pkg(channel, version, builddate, noBuild)