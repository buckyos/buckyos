import os
import sys
import tempfile
import shutil
import subprocess
import perpare_installer
from datetime import datetime

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
rootfs_dir = os.path.join(src_dir, "rootfs")
installer_script = os.path.join(src_dir, "publish", "installer.iss")

def make_installer(version, onlyBuild, noBuild):
    root_dir = os.path.join(src_dir, "buckyos_installer")
    dest_dir = os.path.join(root_dir, "rootfs")
    if not onlyBuild:
        if os.path.exists(root_dir):
            shutil.rmtree(root_dir)
        os.makedirs(root_dir)
        shutil.copy(installer_script, os.path.join(root_dir, "installer.iss"))

        date = datetime.now().strftime("%Y%m%d")
        perpare_installer.prepare_installer(dest_dir, "nightly", "windows", "amd64", version, date)

        print(f"download home-station...")
        app_bin_dir = os.path.join(dest_dir, "bin", "home-station")
        if not os.path.exists(app_bin_dir):
            print("downloading filebrowser app on windows")
            os.makedirs(app_bin_dir,exist_ok=True)

            import urllib.request
            import zipfile
            [tmp_path, msg] = urllib.request.urlretrieve("https://web3.buckyos.io/static/home-station-win.zip")

            with zipfile.ZipFile(tmp_path, 'r') as zip_ref:
                zip_ref.extractall(app_bin_dir)
            os.remove(tmp_path)
    else:
        shutil.copy(installer_script, os.path.join(root_dir, "installer.iss"))
    
    if not noBuild:
        print(f"run build in {root_dir}")
        subprocess.run(f"iscc /DMyAppVersion=\"{version}\" .\\installer.iss", shell=True, check=True, cwd=root_dir)
        print(f"build installer success at {root_dir}")
        shutil.copy(f"{root_dir}/buckyos-installer-{version}.exe", src_dir)
        print(f"copy installer to {src_dir}")

if __name__ == "__main__":
    print("make sure YOU already run build.py!!!")
    version = "0.3.0"
    onlyBuild = False
    noBuild = False
    for arg in sys.argv:
        if arg == "--only-build":
            onlyBuild = True
        elif arg == "--no-build":
            noBuild = True
        else:
            version = arg
    make_installer(version, onlyBuild, noBuild)