import os
import shutil
import sys
import tempfile
import platform
import platform

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
root_bin_dir = os.path.join(src_dir, "rootfs/bin")
system = platform.system()
ext = ""
if system == "Windows":
    ext = ".exe"
system = platform.system()
ext = ""
if system == "Windows":
    ext = ".exe"

def strip_and_copy_rust_file(rust_target_dir, name, dest, need_dir=False):
    src_file = os.path.join(rust_target_dir, "release", name)
    if need_dir:
        dest = os.path.join(dest, name)
        os.makedirs(dest, exist_ok=True)
    shutil.copy(src_file+ext, dest)

    # no need strip symbol on windows
    if system != "Windows":
        os.system(f"strip {os.path.join(dest, name)}")

def copy_web_apps(src, target):
    dist_dir = os.path.join(src_dir, src, "dist")
    os.makedirs(target, exist_ok=True)
    print(f'copying vite build {dist_dir} to {target}')
    shutil.rmtree(target)
    shutil.copytree(dist_dir, target, copy_function=shutil.copyfile)
    pass

def copy_files(rust_target_dir):
    print("Copying files...")
    # code to copy files
    strip_and_copy_rust_file(rust_target_dir, "node_daemon", root_bin_dir,True)
    strip_and_copy_rust_file(rust_target_dir, "system_config", root_bin_dir, True)
    strip_and_copy_rust_file(rust_target_dir, "verify_hub", root_bin_dir, True)
    strip_and_copy_rust_file(rust_target_dir, "scheduler", root_bin_dir, True)
    strip_and_copy_rust_file(rust_target_dir, "cyfs_gateway", root_bin_dir, True)
    strip_and_copy_rust_file(rust_target_dir, "smb_service", root_bin_dir, True)
    strip_and_copy_rust_file(rust_target_dir, "repo_service", root_bin_dir, True)
    strip_and_copy_rust_file(rust_target_dir, "buckycli", root_bin_dir, True)

    strip_and_copy_rust_file(rust_target_dir, "cyfs_gateway", os.path.join(src_dir, "./web3_bridge"))

    if os.path.exists(os.path.join(src_dir, "./web3_bridge", "web3_gateway"+ext)):
        os.remove(os.path.join(src_dir, "./web3_bridge", "web3_gateway"+ext))
    os.rename(os.path.join(src_dir, "./web3_bridge", "cyfs_gateway"+ext), os.path.join(src_dir, "./web3_bridge", "web3_gateway"+ext))

    shutil.copy(os.path.join(src_dir, "killall.py"), root_bin_dir)

    copy_web_apps("kernel/node_active", os.path.join(root_bin_dir, "node_active"))
    copy_web_apps("apps/control_panel/src", os.path.join(root_bin_dir, "control_panel"))
    copy_web_apps("apps/sys_test", os.path.join(root_bin_dir, "sys_test"))

    print("Files copied successfully!")

if __name__ == "__main__":
    args = sys.argv[1:]
    print("MUST RUN build.py FIRST!!")
    if len(args) == 0:
        print("NEED ARGUMENT: amd64|aarch64")
        exit(1)
    if len(args) > 0:
        temp_dir = tempfile.gettempdir()
        project_name = "buckyos"
        target_dir = os.path.join(temp_dir, "rust_build", project_name)
        if args[0] == "amd64":
            copy_files(os.path.join(target_dir, "x86_64-unknown-linux-musl"))
        elif args[0] == "aarch64":
            copy_files(os.path.join(target_dir, "aarch64-unknown-linux-musl"))
        else:
            print("Invalid argument: clean|amd64|aarch64")
            exit(1)
