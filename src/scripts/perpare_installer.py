# 从make_deb的逻辑整理而来，平台无关的安装包前的准备工作
# 1. 拷贝rootfs到某个指定的文件夹，一般是/tmp下的某个Installer相关文件夹
# 2. 清除掉拷贝后的rootfs/bin，之后要重新组织
# 3. 制作metadb, 并重新"install"bin文件夹
# 4. 整理和移除不需要的文件
import os
import shutil
import json
import subprocess
import time
import glob

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
rootfs_dir = os.path.join(src_dir, "rootfs")
base_meta_db_url = "https://buckyos.ai/ndn/repo/meta_index.db/content"
bucky_cli_path = os.path.join(src_dir, "rootfs","bin", "buckycli","buckycli")

def prepare_meta_db(rootfs_dir):
    # 1 download base meta db
    print(f"# download base meta db from {base_meta_db_url}")
    os.makedirs(os.path.join(rootfs_dir, "local", "node_daemon", "root_pkg_env","pkgs"), exist_ok=True)
    root_env_db_path = os.path.join(rootfs_dir, "local", "node_daemon", "root_pkg_env","pkgs","meta_index.db")
    subprocess.run(["wget",base_meta_db_url,"-O",root_env_db_path], check=True)
    print(f"# download base meta db to {root_env_db_path}")
    # 2 scan packed pkgs dir, add pkg_meta_info to meta db
    packed_pkgs_dir = "/tmp/buckyos_pkg_out/"
    print(f"# packed_pkgs_dir: {packed_pkgs_dir}")
    pkg_items = glob.glob(os.path.join(packed_pkgs_dir, "*"))
    for pkg_item in pkg_items:
        print(f"# add pkg_meta_info to meta db from {pkg_item}")
        item_path = os.path.join(packed_pkgs_dir, pkg_item)
        if os.path.isdir(item_path):
            item_path = os.path.join(item_path, "pkg_meta.jwt")
            if os.path.exists(item_path):
                subprocess.run([bucky_cli_path,"set_pkg_meta",item_path,root_env_db_path], check=True)
                print(f"# add pkg_meta_info to meta db from {item_path}")
        else:
            if item_path.endswith(".jwt"):
                subprocess.run([bucky_cli_path,"set_pkg_meta",item_path,root_env_db_path], check=True)
                print(f"# add pkg_meta_info to meta db from {item_path}")

    fileobj_path = os.path.join(rootfs_dir, "local", "node_daemon", "root_pkg_env","pkgs", "meta_index.db.fileobj")
    fileobj = json.load(open(fileobj_path))
    
    current_time = int(time.time())
    fileobj["create_time"] = current_time
    json.dump(fileobj, open(fileobj_path, "w"))
    fileobj_path = os.path.join(rootfs_dir, "data", "repo-service", "default_meta_index.db.fileobj")
    json.dump(fileobj, open(fileobj_path, "w"))
    print(f"# update fileobj create_time to {current_time} for {fileobj_path}")
    os.makedirs(os.path.join(rootfs_dir, "bin", "pkgs"), exist_ok=True)
    shutil.copy(root_env_db_path, os.path.join(rootfs_dir, "bin", "pkgs", "meta_index.db"))
    shutil.copy(root_env_db_path, os.path.join(rootfs_dir, "data", "repo-service", "default_meta_index.db"))
    print(f"# save meta db to {os.path.join(rootfs_dir, 'bin', 'pkgs', 'meta_index.db')}")
    

def install_pkgs_to_bin(bin_path):
    pkg_list = [
        "buckycli",
        "control_panel",
        "cyfs_gateway",
        "node_active",
        "node_daemon",
        "repo_service",
        "scheduler",
        "smb_service",
        "system_config",
        "verify_hub",
        "app_loader",
        "sys_test",
    ]

    for pkg_id in pkg_list:
        print(f"> buckycli install_pkg {pkg_id} --env {bin_path}")
        subprocess.run([bucky_cli_path,"install_pkg",pkg_id,"--env",bin_path], check=True)
        print(f"# install {pkg_id} success.")

def prepare_installer(target_dir, os_name, arch):
    if not os.path.exists(target_dir):
        os.makedirs(target_dir)
    shutil.copytree(rootfs_dir, target_dir, dirs_exist_ok=True)
    print(f"# copy rootfs to {target_dir}")

    bin_dir = os.path.join(target_dir, "bin")
    shutil.rmtree(bin_dir)
    os.makedirs(bin_dir)
    print("# remove /opt/buckyos/bin dir and create it again")
    # write pkg.cfg.json to bin_dir
    pkg_cfg_path = os.path.join(src_dir, "publish", "buckyos_pkgs","pkg.cfg.json")
    pkg_cfg = json.load(open(pkg_cfg_path))
    pkg_cfg["prefix"] = f"nightly-{os_name}-{arch}"
    old_parent = pkg_cfg["parent"]
    pkg_cfg["parent"] = None
    json.dump(pkg_cfg, open(os.path.join(bin_dir, "pkg.cfg.json"), "w"))
    print(f"# write pkg.cfg.json to {bin_dir} OK ")

    prepare_meta_db(target_dir)
    print(f"# prepare meta db to {target_dir}")
    install_pkgs_to_bin(bin_dir)
    print(f"# install pkgs to {bin_dir}")
    pkg_cfg["parent"] = old_parent
    json.dump(pkg_cfg, open(os.path.join(bin_dir, "pkg.cfg.json"), "w"))

    os.remove(os.path.join(bin_dir, "pkgs", "meta_index.db"))
    print(f"# remove meta_index.db from {bin_dir}")

    clean_dir = os.path.join(target_dir, "etc")
    print(f"clean all .pem and .toml files and start_config.json in {clean_dir}")
    for file in glob.glob(os.path.join(clean_dir, "*.pem")):
        os.remove(file)
    for file in glob.glob(os.path.join(clean_dir, "*.toml")):
        os.remove(file)
    os.remove(os.path.join(clean_dir, "start_config.json"))
    os.remove(os.path.join(clean_dir, "node_identity.json"))
    for file in glob.glob(os.path.join(clean_dir, "*.zone.json")):
        os.remove(file)
    os.remove(os.path.join(clean_dir, "scheduler", "boot.template.toml"))
    shutil.move(os.path.join(clean_dir, "scheduler", "nightly.template.toml"), os.path.join(clean_dir, "scheduler", "boot.template.toml"))
    shutil.move(os.path.join(clean_dir, "machine.json"), os.path.join(clean_dir, "machine_config.json"))