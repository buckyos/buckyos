import os
import shutil
import platform
import platform

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")

install_root_dir = ""

if platform.system() == "Windows":
    install_root_dir = os.path.join(os.path.expandvars("%AppData%"), "buckyos")
else:
    install_root_dir = "/opt/buckyos"

def set_data_dir_permissions():
    if platform.system() != "Windows":  # Windows 不需要设置权限
        import pwd
        import grp
        
        # 获取 SUDO_USER 环境变量，这是实际运行 sudo 的用户
        real_user = os.environ.get('SUDO_USER')
        if real_user:
            data_dir = os.path.join(install_root_dir, "data")
            if not os.path.exists(data_dir):
                os.makedirs(data_dir)
            
            # 获取真实用户的 uid 和 gid
            uid = pwd.getpwnam(real_user).pw_uid
            gid = pwd.getpwnam(real_user).pw_gid
            
            # 递归设置目录权限
            for root, dirs, files in os.walk(data_dir):
                os.chown(root, uid, gid)
                for d in dirs:
                    os.chown(os.path.join(root, d), uid, gid)
                for f in files:
                    os.chown(os.path.join(root, f), uid, gid)
            
            # 设置目录权限为 755 (rwxr-xr-x)
            os.chmod(data_dir, 0o755)

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

    # 在安装完成后设置数据目录权限
    set_data_dir_permissions()

if __name__ == "__main__":
    import sys
    install_all = "--all" in sys.argv
    print(f"installing to {install_root_dir}, install_all: {install_all}")
    install(install_all)