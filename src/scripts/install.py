import os
import shutil
import platform

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")

install_root_dir = ""

if platform.system() == "Windows":
    install_root_dir = os.path.join(os.path.expandvars("%AppData%"), "buckyos")
else:
    install_root_dir = "/opt/buckyos"

def install(install_all=False):
    if install_root_dir == "":
        print("Unknown platform, not support install, skip.")
        return
    # if /opt/buckyos not exist, copy rootfs to /opt/buckyos
    print(f"installing to {install_root_dir}")
    if not os.path.exists(install_root_dir):
        install_all = True
    
    if install_all:
        print(f'copying rootfs to {install_root_dir}')
        if os.path.exists(install_root_dir):
            # 删除目标目录下的所有子项
            for item in os.listdir(install_root_dir):
                item_path = os.path.join(install_root_dir, item)
                print(f'removing {item_path}')
                if os.path.isfile(item_path):
                    os.remove(item_path)
                elif os.path.isdir(item_path):
                    shutil.rmtree(item_path)
            # 复制rootfs下的所有sub_items
            for item in os.listdir(os.path.join(src_dir, "rootfs")):
                item_path = os.path.join(src_dir, "rootfs", item)
                print(f'copying {item_path} to {install_root_dir}')
                if os.path.isfile(item_path):
                    shutil.copy(item_path, install_root_dir)
                elif os.path.isdir(item_path):
                    shutil.copytree(item_path, os.path.join(install_root_dir, item))
        else:
            shutil.copytree(os.path.join(src_dir, "rootfs"), install_root_dir)
    else:
        bin_dir = os.path.join(install_root_dir, "bin")

        print(f'updating files in {bin_dir}')
        if os.path.exists(bin_dir):
            shutil.rmtree(bin_dir)
        #just update bin
        shutil.copytree(os.path.join(src_dir, "rootfs/bin"), bin_dir)

    if platform.system() == "Windows":
        app_bin_dir = os.path.join(install_root_dir, "bin", "home-station")
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
        print("pulling filebrowser docker image...")
        os.system("docker pull filebrowser/filebrowser:s6")

if __name__ == "__main__":
    import sys
    install_all = "--all" in sys.argv
    print(f"installing to {install_root_dir}, install_all: {install_all}")
    install(install_all)