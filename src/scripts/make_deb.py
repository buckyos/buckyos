import os
import sys
import tempfile
import shutil
import subprocess

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

def make_deb(architecture, version):
    print(f"make deb with architecture: {architecture}, version: {version}")
    deb_root_dir = os.path.join(temp_dir, "deb_build")
    print(f"deb_root_dir: {deb_root_dir}")
    deb_dir = os.path.join(deb_root_dir, architecture)
    if os.path.exists(deb_dir):
        shutil.rmtree(deb_dir)
    shutil.copytree(publish_dir, deb_dir)

    adjust_control_file(deb_dir, version, architecture)
    rootfs_dir = os.path.join(src_dir, "rootfs")
    dest_dir = os.path.join(deb_dir, "opt", "buckyos")
    shutil.copytree(rootfs_dir, dest_dir, dirs_exist_ok=True)
    print(f"copy rootfs to {dest_dir}")

    print(f"run: chmod -R 755 {deb_dir}")
    subprocess.run(["chmod", "-R", "755", deb_dir], check=True)

    clean_dir = os.path.join(dest_dir, "etc")
    print(f"clean all .pem and .toml files in {clean_dir}")
    subprocess.run("rm -f *.pem *.toml", shell=True, check=True, cwd=clean_dir)
    subprocess.run([f"dpkg-deb --build {architecture}"], shell=True, check=True, cwd=deb_root_dir)
    print(f"build deb success at {deb_dir}")
    shutil.copy(f"{deb_root_dir}/{architecture}.deb", os.path.join(src_dir, f"buckyos_{architecture}.deb"))
    print(f"copy deb to {src_dir}")

if __name__ == "__main__":
    print("make sure YOU already run build.py!!!")
    architecture = "amd64"
    version = "0.5.0"

    if len(sys.argv) > 1:
        architecture = sys.argv[1]

    if len(sys.argv) > 2:
        version = sys.argv[2]
        
    make_deb(architecture, version)