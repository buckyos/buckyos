import json
import os
import sys
import tempfile
import shutil
import subprocess
import glob
import time

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
publish_dir = os.path.join(src_dir, "publish", "deb_template")
base_meta_db_url = "http://buckyos.ai/ndn/repo/meta_index.db/content"
bucky_cli_path = os.path.join(src_dir, "rootfs","bin", "buckycli","buckycli")

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

temp_dir = "/tmp/"


def prepare_meta_db(rootfs_dir):
    # 1 download base meta db
    print("# download base meta db from {base_meta_db_url}")
    root_env_db_path = os.path.join(rootfs_dir, "local", "node_daemon", "root_pkg_env",".pkgs","meta_index.db")
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

    fileobj_path = os.path.join(rootfs_dir, "local", "node_daemon", "root_pkg_env",".pkgs", "meta_index.db.fileobj")
    fileobj = json.load(open(fileobj_path))
    fileobj["create_time"] = time.time()
    json.dump(fileobj, open(fileobj_path, "w"))
    fileobj_path = os.path.join(rootfs_dir, "data", "repo_service", "default_meta_index.db.fileobj")
    json.dump(fileobj, open(fileobj_path, "w"))
    print(f"# update fileobj create_time to {time.time()} for {fileobj_path}")
    os.makedirs(os.path.join(rootfs_dir, "bin", ".pkgs"), exist_ok=True)
    shutil.copy(root_env_db_path, os.path.join(rootfs_dir, "bin", ".pkgs", "meta_index.db"))
    shutil.copy(root_env_db_path, os.path.join(rootfs_dir, "data", "repo_service", "default_meta_index.db"))
    print(f"# save meta db to {os.path.join(rootfs_dir, 'bin', '.pkgs', 'meta_index.db')}")
    

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



def make_deb(architecture, version):
    print(f"make deb with architecture: {architecture}, version: {version}")
    deb_root_dir = os.path.join(temp_dir, "deb_build")
    deb_dir = os.path.join(deb_root_dir, architecture)
    if os.path.exists(deb_dir):
        shutil.rmtree(deb_dir)
    shutil.copytree(publish_dir, deb_dir)

    adjust_control_file(deb_dir, version, architecture)
    rootfs_dir = os.path.join(src_dir, "rootfs")
    dest_dir = os.path.join(deb_dir, "opt", "buckyos")
    shutil.copytree(rootfs_dir, dest_dir, dirs_exist_ok=True)
    print(f"# copy rootfs to {dest_dir}")
    
    bin_dir = os.path.join(dest_dir, "bin")
    shutil.rmtree(bin_dir)
    os.makedirs(bin_dir)
    print("# remove /opt/buckyos/bin dir and create it again")
    # write pkg.cfg.json to bin_dir
    pkg_cfg_path = os.path.join(src_dir, "publish", "buckyos_pkgs","pkg.cfg.json")
    pkg_cfg = json.load(open(pkg_cfg_path))
    pkg_cfg["prefix"] = f"nightly-linux-{architecture}"
    old_parent = pkg_cfg["parent"]
    pkg_cfg["parent"] = None
    json.dump(pkg_cfg, open(os.path.join(bin_dir, "pkg.cfg.json"), "w"))
    print(f"# write pkg.cfg.json to {bin_dir} OK ")

    prepare_meta_db(dest_dir)
    print(f"# prepare meta db to {dest_dir}")
    install_pkgs_to_bin(bin_dir)
    print(f"# install pkgs to {bin_dir}")
    pkg_cfg["parent"] = old_parent
    json.dump(pkg_cfg, open(os.path.join(bin_dir, "pkg.cfg.json"), "w"))

    os.remove(os.path.join(bin_dir, ".pkgs", "meta_index.db"))
    print(f"# remove meta_index.db from {bin_dir}")

    print(f"run: chmod -R 755 {deb_dir}")
    subprocess.run(["chmod", "-R", "755", deb_dir], check=True)

    clean_dir = os.path.join(dest_dir, "etc")
    print(f"clean all .pem and .toml files and start_config.json in {clean_dir}")
    subprocess.run("rm -f *.pem *.toml", shell=True, check=True, cwd=clean_dir)
    subprocess.run("rm -f start_config.json", shell=True, check=True, cwd=clean_dir)
    subprocess.run("rm -f node_identity.json", shell=True, check=True, cwd=clean_dir)
    subprocess.run("rm -f *.zone.json", shell=True, check=True, cwd=clean_dir)
    subprocess.run("rm -f scheduler/boot.template.toml", shell=True, check=True, cwd=clean_dir)
    subprocess.run("mv scheduler/nightly.template.toml scheduler/boot.template.toml", shell=True, check=True, cwd=clean_dir)
    subprocess.run("mv machine.json machine_config.json", shell=True, check=True, cwd=clean_dir)
    subprocess.run([f"dpkg-deb --build {architecture}"], shell=True, check=True, cwd=deb_root_dir)
    print(f"build deb success at {deb_dir}")
    shutil.copy(f"{deb_root_dir}/{architecture}.deb", os.path.join(src_dir, f"buckyos_{architecture}.deb"))
    print(f"copy deb to {src_dir}")

if __name__ == "__main__":
    print("make sure YOU already run build.py!!!")
    architecture = "amd64"
    #architecture = "aarch64"
    version = "0.4.0"

    if len(sys.argv) > 1:
        architecture = sys.argv[1]

    if len(sys.argv) > 2:
        version = sys.argv[2]
    make_deb(architecture, version)