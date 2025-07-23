import os
import shutil
import platform

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")

install_root_dir = ""

pre_install_apps = [
    {
        "app_id": "buckyos-filebrowser",
        "base_url": "https://github.com/buckyos/filebrowser/releases/download/",
    }
]

if platform.system() == "Windows":
    install_root_dir = os.path.join(os.path.expandvars("%AppData%"), "buckyos")
else:
    install_root_dir = "/opt/buckyos"

def set_data_dir_permissions():
    if platform.system() != "Windows":  # Windows doesn't need permission setting
        import pwd
        import grp
        
        # Get SUDO_USER environment variable, which is the actual user running sudo
        real_user = os.environ.get('SUDO_USER')
        if real_user:
            data_dir = os.path.join(install_root_dir, "data")
            if not os.path.exists(data_dir):
                os.makedirs(data_dir)
            
            # Get the real user's uid and gid
            uid = pwd.getpwnam(real_user).pw_uid
            gid = pwd.getpwnam(real_user).pw_gid
            
            # Recursively set directory permissions
            for root, dirs, files in os.walk(data_dir):
                os.chown(root, uid, gid)
                for d in dirs:
                    os.chown(os.path.join(root, d), uid, gid)
                for f in files:
                    os.chown(os.path.join(root, f), uid, gid)
            
            # Set directory permissions to 755 (rwxr-xr-x)
            os.chmod(data_dir, 0o755)

def unzip_to_dir(zip_path, target_dir):
    """Extract zip file to target directory, content directly in target directory"""
    if not os.path.exists(target_dir):
        os.makedirs(target_dir)
    
    # First extract to temporary directory
    temp_dir = os.path.join(os.path.dirname(zip_path), f"temp_{os.path.basename(zip_path)}")
    shutil.unpack_archive(zip_path, temp_dir)
    
    # Move extracted content to target directory
    extracted_dir = os.path.join(temp_dir, os.listdir(temp_dir)[0])
    for item in os.listdir(extracted_dir):
        src_path = os.path.join(extracted_dir, item)
        dst_path = os.path.join(target_dir, item)
        if os.path.isfile(src_path):
            shutil.copy2(src_path, dst_path)
        elif os.path.isdir(src_path):
            shutil.copytree(src_path, dst_path)
    
    # Clean up temporary directory
    shutil.rmtree(temp_dir)
    print(f"Extraction completed: {zip_path} -> {target_dir}")

def download_file(url, filepath):
    """Cross-platform file download using system built-in commands"""
    system = platform.system().lower()
    
    if system == "windows":
        # Windows uses PowerShell's Invoke-WebRequest
        cmd = f'powershell -Command "Invoke-WebRequest -Uri \'{url}\' -OutFile \'{filepath}\' -UseBasicParsing"'
    elif system == "darwin":  # macOS
        # macOS uses curl
        cmd = f"curl -L -o '{filepath}' '{url}'"
    else:  # Linux
        # Linux prioritizes wget, falls back to curl if not available
        cmd = f"wget -c -L -O '{filepath}' '{url}'"
    
    print(f"Downloading: {url}")
    print(f"Saving to: {filepath}")
    print(f"Executing command: {cmd}")
    
    result = os.system(cmd)
    if result == 0:
        print(f"Download completed: {filepath}")
        return True
    else:
        print(f"Download failed, exit code: {result}")
        # If wget fails, try using curl
        if system not in ["windows", "darwin"]:
            print("Trying curl...")
            curl_cmd = f"curl -L -o '{filepath}' '{url}'"
            result = os.system(curl_cmd)
            if result == 0:
                print(f"Download completed with curl: {filepath}")
                return True
        
        return False

def copy_configs(config_group_name):
    etc_dir = os.path.join(install_root_dir, "etc")
    configs_dir = os.path.join(src_dir, "scripts","configs_group",config_group_name)
    print(f"Copying configs from {configs_dir} to {etc_dir}")
    for config_file in os.listdir(configs_dir):
        config_path = os.path.join(configs_dir, config_file)
        if os.path.isfile(config_path):
            shutil.copy(config_path, etc_dir)
            print(f"Copied file {config_path} to {etc_dir}")
        #elif os.path.isdir(config_path):
        #    shutil.copytree(config_path, os.path.join(etc_dir, config_file))
        #    print(f"Copied directory {config_path} to {etc_dir}")

def install_apps():
    temp_dir = os.environ.get('TEMP') or os.environ.get('TMP') or '/tmp'
    download_dir = os.path.join(temp_dir, "buckyos-apps")
    version = os.path.join(src_dir, "VERSION")
    with open(version, "r") as f:
        version = f.read().strip()
    print(f"current version: {version},download dir: {download_dir}")
    
    # check and download app_pkg_zips
    # unzip to dest dir
    os_name = platform.system().lower()
    arch = platform.machine().lower()

    if not os.path.exists(download_dir):
        os.makedirs(download_dir)
    #nightly-apple-aarch64.buckyos-filebrowser-bin.zip
    preifx = f"nightly-{os_name}-{arch}"
    img_prefix = f"nightly-linux-{arch}"
    print(f"app prefix is {preifx}")
    for app in pre_install_apps:
        app_full_id = f"{preifx}.{app['app_id']}-bin.zip"
        download_url = f"{app['base_url']}{version}/{app_full_id}"
        download_path = os.path.join(download_dir, f"{app['app_id']}-bin.zip")
        if download_file(download_url, download_path):
            print(f"download {app_full_id} OK")
            unzip_dir = os.path.join(install_root_dir, "bin", f"{app['app_id']}-bin")
            unzip_to_dir(download_path, unzip_dir)
            print(f"unzip {app_full_id} OK")
        else:
            print(f"download {app_full_id} FAILED")

        app_img_full_id = f"{img_prefix}.{app['app_id']}-img.zip"
        download_url = f"{app['base_url']}{version}/{app_img_full_id}"
        download_path = os.path.join(download_dir, f"{app['app_id']}-img.zip")
        if download_file(download_url, download_path):
            print(f"download {app_img_full_id} OK")
            unzip_dir = os.path.join(install_root_dir, "bin", f"{app['app_id']}-img")
            unzip_to_dir(download_path, unzip_dir)
            print(f"unzip {app_img_full_id} OK")
        else:
            print(f"download {app_img_full_id} FAILED")

        print(f"install {app['app_id']} OK")

    return

def install(install_all=False):
    if install_root_dir == "":
        print("Unknown platform, installation not supported, skipping.")
        return
    # if /opt/buckyos doesn't exist, copy rootfs to /opt/buckyos
    print(f"Installing to {install_root_dir}")
    etc_dir = os.path.join(install_root_dir, "etc")
    if not os.path.exists(etc_dir):
        install_all = True
    
    if install_all:
        print(f'Copying rootfs to {install_root_dir}')
        
        if os.path.exists(install_root_dir):
            # Remove all items in target directory
            for item in os.listdir(install_root_dir):
                item_path = os.path.join(install_root_dir, item)
                print(f'Removing {item_path}')
                if os.path.isfile(item_path):
                    os.remove(item_path)
                elif os.path.isdir(item_path):
                    shutil.rmtree(item_path)
            # Copy all sub_items from rootfs
            for item in os.listdir(os.path.join(src_dir, "rootfs")):
                item_path = os.path.join(src_dir, "rootfs", item)
                print(f'Copying {item_path} to {install_root_dir}')
                if os.path.isfile(item_path):
                    shutil.copy(item_path, install_root_dir)
                elif os.path.isdir(item_path):
                    shutil.copytree(item_path, os.path.join(install_root_dir, item))
        else:
            shutil.copytree(os.path.join(src_dir, "rootfs"), install_root_dir)
    else:
        bin_dir = os.path.join(install_root_dir, "bin")

        print(f'Updating files in {bin_dir}')
        if os.path.exists(bin_dir):
            print(f'Removing {bin_dir}')
            shutil.rmtree(bin_dir)
        # Just update bin
        print(f'Copying {os.path.join(src_dir, "rootfs","bin")} => {bin_dir}')
        shutil.copytree(os.path.join(src_dir, "rootfs","bin"), bin_dir)

    # Set data directory permissions after installation
    set_data_dir_permissions()

if __name__ == "__main__":
    import sys
    install_all = "--all" in sys.argv
    print(f"Installing to {install_root_dir}, install_all: {install_all}")
    install(install_all)
    install_apps()