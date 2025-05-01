import os
import sys
import tempfile
import shutil
import subprocess
from datetime import datetime
import perpare_installer

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
publish_dir = os.path.join(src_dir, "publish", "deb_template")

def adjust_control_file(dest_dir, new_version, architecture):
    control_file = os.path.join(dest_dir, "DEBIAN/control")
    f = open(control_file, "r")
    content = f.read()
    f.close()
    content = content.replace("{{package version here}}", new_version)
    content = content.replace("{{architecture}}", architecture)
    f = open(control_file, "w")
    f.write(content)
    f.close()

temp_dir = tempfile.gettempdir()

def make_deb(architecture, version, builddate):
    print(f"make deb with architecture: {architecture}, version: {version}")
    deb_root_dir = os.path.join(temp_dir, "deb_build")
    print(f"deb_root_dir: {deb_root_dir}")
    deb_dir = os.path.join(deb_root_dir, architecture)
    if os.path.exists(deb_dir):
        shutil.rmtree(deb_dir)
    shutil.copytree(publish_dir, deb_dir)

    adjust_control_file(deb_dir, version, architecture)
    dest_dir = os.path.join(deb_dir, "opt", "buckyos")

    perpare_installer.prepare_installer(dest_dir, "nightly", "linux", architecture, version, builddate)

    print(f"run: chmod -R 755 {deb_dir}")
    subprocess.run(["chmod", "-R", "755", deb_dir], check=True)

    subprocess.run([f"dpkg-deb --build {architecture}"], shell=True, check=True, cwd=deb_root_dir)
    print(f"build deb success at {deb_dir}")
    
    dst_deb_path = os.path.join(src_dir, f"buckyos-{architecture}-{version}.deb")
    shutil.copy(f"{deb_root_dir}/{architecture}.deb", dst_deb_path)
    print(f"copy deb to {dst_deb_path}")

if __name__ == "__main__":
    print("make sure YOU already run build.py!!!")
    architecture = "amd64"
    #architecture = "aarch64"
    version = "0.4.0"
    builddate = datetime.now().strftime("%Y%m%d")

    if len(sys.argv) > 1:
        architecture = sys.argv[1]

    if len(sys.argv) > 2:
        version = sys.argv[2]

    if len(sys.argv) > 3:
        builddate = sys.argv[3]

    if architecture == "x86_64":
        architecture = "amd64"
    make_deb(architecture, version, builddate)