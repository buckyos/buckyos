import os
import shutil
import platform

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")

install_root_dir = ""

if platform.system() == "Windows":
    install_root_dir = os.path.join(os.path.expandvars("%AppData%"), "buckyos")
else:
    install_root_dir = "/opt/buckyos"

def install():
    if install_root_dir == "":
        print("Unknown platform, not support install, skip.")
        return
    # if /opt/buckyos not exist, copy rootfs to /opt/buckyos
    print(f"installing to {install_root_dir}")
    if not os.path.exists(install_root_dir):
        print(f'copying rootfs to {install_root_dir}')
        shutil.copytree(os.path.join(src_dir, "rootfs"), install_root_dir)
    else:
        bin_dir = os.path.join(install_root_dir, "bin")
        print(f'updating files in {bin_dir}')
        if os.path.exists(bin_dir):
            shutil.rmtree(bin_dir)
        #just update bin
        shutil.copytree(os.path.join(src_dir, "rootfs/bin"), bin_dir)

if __name__ == "__main__":
    install()