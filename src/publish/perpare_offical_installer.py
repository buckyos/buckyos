# 从make_deb的逻辑整理而来，平台无关的安装包前的准备工作
# 1. 拷贝rootfs到某个指定的文件夹，一般是/tmp下的某个Installer相关文件夹
# 2. 清除掉拷贝后的rootfs/bin，之后要重新组织
# 3. 调用perpare_packages，准备好新的PackageMeta
# 4. 从官方源下载现在的meta db文件（这一步可以跳过?)
# 5. 将新版本的PackageMeta添加进本地的meta db里, 并重新"install"bin文件夹 --> 会导致产生符号链接
# 4. 整理和移除不需要的文件
import os
import shutil
import json
import subprocess
import time
import glob
import platform

from urllib.request import urlretrieve

publish_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)))
buckyosci_base = os.environ.get("BUCKYOS_BUILD_ROOT", "/opt/buckyosci")
packed_pkg_base_dir = os.path.join(buckyosci_base, "pack_pkgs")
rootfs_base_dir = packed_pkg_base_dir
base_meta_db_url = "https://buckyos.ai/ndn/repo/meta_index.db/content"

# 如果有定义BUCKYCLI_PATH环境变量，就使用这个变量作为CLI的执行文件
buckycli_path = os.getenv("BUCKYCLI_PATH", "/opt/buckyos/bin/buckycli/buckycli")

def install_pkg(pkg_name, target_dir, prefix, version):
    pkg_full_name = f"{prefix}.{pkg_name}"
    pkg_dir = os.path.join(packed_pkg_base_dir,version,pkg_full_name)
    print(f"# install pkg {pkg_dir} to {target_dir}...")

    meta_obj_id_file = os.path.join(pkg_dir, "meta_obj_id")
    if not os.path.exists(meta_obj_id_file):
        print(f"# meta_obj_id_file not found in {pkg_dir}")
        return
    meta_obj_id = open(meta_obj_id_file).read().strip()
    target_pkg_dir = os.path.join(target_dir, pkg_name)

    pkg_file = os.path.join(pkg_dir, f"{pkg_full_name}#{version}.tar.gz")
    if not os.path.exists(pkg_file):
        print(f"# pkg_file not found in {pkg_dir}")
        return
    os.makedirs(target_pkg_dir, exist_ok=True)
    #解压到 bin/pkgs/pkg_name/$meta_obj_id 目录下
    subprocess.run(["tar", "-xzf", pkg_file, "-C", target_pkg_dir], check=True)
    print(f"# install pkg {pkg_file} to {target_pkg_dir} done")


def prepare_bin_package(src_pkg_dir, prefix, version, target_dir):
    pack_pkg_items = glob.glob(os.path.join(src_pkg_dir,f"{prefix}*"))
    print(f"# prepare_bin_package: {src_pkg_dir} ({prefix}*)")
    install_pkg("node_daemon",target_dir,prefix,version)
    #install_pkg("node_active",target_dir,prefix,version)
    #install_pkg("buckycli",target_dir,prefix,version)
    # for pack_pkg_item in pack_pkg_items:
    #     if os.path.isdir(pack_pkg_item):
    #         #nightly-windows-amd64.app_loader ,app_loader is pkg_name
    #         pkg_name = pack_pkg_item.split(".")[-1]
    #         install_pkg(pkg_name, target_dir, prefix, version)


def prepare_named_mgr_data(rootfs_dir,src_pkg_dir,prefix):
    pack_pkg_items = glob.glob(os.path.join(src_pkg_dir,f"{prefix}*"))
    named_data_dir = os.path.join(rootfs_dir, "data", "ndn")
    for pack_pkg_item in pack_pkg_items:
        if os.path.isdir(pack_pkg_item):
            print(f"# prepare_named_mgr_data: add {pack_pkg_item} to installer")
            #nightly-windows-amd64.app_loader ,app_loader is pkg_name
            chunk_files = glob.glob(os.path.join(pack_pkg_item,"*.tar.gz"))
            for chunk_file in chunk_files:
                subprocess.run([buckycli_path,"create_chunk",chunk_file,named_data_dir], check=True)

def prepare_meta_db(rootfs_dir,packed_pkgs_dir):
    meta_db_path = os.path.join(packed_pkgs_dir,"meta_index.db")
    meta_db_fileobj_path = os.path.join(packed_pkgs_dir,"meta_index.db.fileobj")
    if not os.path.exists(meta_db_path):
        print(f"!!! meta_db_path not found in {packed_pkgs_dir}")
        return
    if not os.path.exists(meta_db_fileobj_path):
        print(f"!!! meta_db_fileobj_path not found in {packed_pkgs_dir}")
        return
    
    os.makedirs(os.path.join(rootfs_dir, "local", "node_daemon", "root_pkg_env","pkgs"), exist_ok=True)
    root_env_db_path = os.path.join(rootfs_dir, "local", "node_daemon", "root_pkg_env","pkgs","meta_index.db")
    shutil.copyfile(meta_db_path, root_env_db_path)
    # subprocess.run(["wget",base_meta_db_url,"-O",root_env_db_path], check=True)
    print(f"# copy {meta_db_path} => {root_env_db_path}")

    fileobj_path = os.path.join(rootfs_dir, "local", "node_daemon", "root_pkg_env","pkgs", "meta_index.db.fileobj")
    try: 
        shutil.copy(meta_db_fileobj_path, fileobj_path)
    except shutil.SameFileError:
        pass  # 如果文件已经存在且相同，则忽略错误
    print(f"# copy {meta_db_fileobj_path} => {fileobj_path}")

    repo_meta_db_path = os.path.join(rootfs_dir, "data", "repo-service", "default_meta_index.db")
    shutil.copyfile(meta_db_path, repo_meta_db_path)
    print(f"# copy {meta_db_path} => {repo_meta_db_path}")
    
    fileobj_path = os.path.join(rootfs_dir, "data", "repo-service", "default_meta_index.db.fileobj")
    shutil.copyfile(meta_db_fileobj_path, fileobj_path)
    print(f"# copy {meta_db_fileobj_path} => {fileobj_path}")


def prepare_rootfs_for_installer(target_dir, os_name, arch, version):
    rootfs_id = f"buckyos-{os_name}-{arch}"
    rootfs_dir = os.path.join(rootfs_base_dir, version, rootfs_id)
    if not os.path.exists(target_dir):
        os.makedirs(target_dir)
    print(f"# copy rootfs: {rootfs_dir} => {target_dir}")    
    shutil.copytree(rootfs_dir, target_dir, dirs_exist_ok=True)
    
    # install pkg to rootfs/bin
    bin_dir = os.path.join(target_dir, "bin")

    # write pkg.cfg.json to bin_dir
    pkg_cfg_path = os.path.join(publish_dir,  "buckyos_pkgs","pkg.cfg.json")
    pkg_cfg = json.load(open(pkg_cfg_path))
    pkg_cfg["prefix"] = f"nightly-{os_name}-{arch}"
    json.dump(pkg_cfg, open(os.path.join(bin_dir, "pkg.cfg.json"), "w"))
    print(f"# write pkg.cfg.json to {bin_dir} OK ")

    packed_pkgs_dir = os.path.join(packed_pkg_base_dir, version)
    prepare_bin_package(packed_pkgs_dir,f"nightly-{os_name}-{arch}", version, bin_dir)
    # if os_name == "windows":
    #     pkg_cfg["parent"] = "c:\\buckyos\\local\\node_daemon\\root_pkg_env"
    # else:
    #     pkg_cfg["parent"] = "/opt/buckyos/local/node_daemon/root_pkg_env"
    # json.dump(pkg_cfg, open(os.path.join(bin_dir, "pkg.cfg.json"), "w"))
    prepare_meta_db(target_dir,packed_pkgs_dir)
    prepare_named_mgr_data(target_dir,packed_pkgs_dir,f"nightly-{os_name}-{arch}")
    
    
    # clean up etc dir
    clean_dir = os.path.join(target_dir, "etc")
    print(f"clean all .pem and .toml files and start_config.json in {clean_dir}")
    for file in glob.glob(os.path.join(clean_dir, "*.pem")):
        os.remove(file)

    start_config_file = os.path.join(clean_dir, "start_config.json")
    if os.path.exists(start_config_file):
        os.remove(start_config_file)
    node_identity_file = os.path.join(clean_dir, "node_identity.json")
    if os.path.exists(node_identity_file):
        os.remove(node_identity_file)
    machine_file = os.path.join(clean_dir, "machine.json")
    if os.path.exists(machine_file):
        os.remove(machine_file)

# for test
if __name__ == "__main__":
    target_dir = "/tmp/buckyos-installer"
    os_name = "linux"
    arch = "amd64"
    version = "0.4.0-250724"
    print(f"TEST perpare_offical_installer.py start... {os_name}-{arch}-{version}")
    if os.path.exists(target_dir):
        shutil.rmtree(target_dir)
    os.makedirs(target_dir)
    prepare_rootfs_for_installer(target_dir, os_name, arch, version)
