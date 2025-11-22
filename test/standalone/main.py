

# 测试入口，支持传入版本号参数。如果传入了版本号，依赖download.py下载特定版本。如果不传入版本号，认为rootfs下有已经编译好的版本
import sys
import platform
import os
import shutil
import subprocess
import tempfile
import traceback

temp_root_dir = tempfile.mkdtemp(prefix="buckyos_standalone_test_")
print(f"Perpareing temporary root directory: {temp_root_dir}")

print("Detecting system architecture...")
ext = ""
architecture = platform.machine().lower()
if architecture == "x86_64":
    architecture = "amd64"

system_os = platform.system().lower()
if system_os == "darwin":
    system_os = "apple"
elif system_os == "windows":
    ext=".exe"

# 如果是linux，使用dev配置, 否则使用dev_no_docker配置
if system_os == "linux":
    group_name = "dev"
else:
    group_name = "dev_no_docker"

print(f"Detected OS: {system_os}, Architecture: {architecture}, executable extension: '{ext}', using config group: {group_name}")

def main():
    root_dir = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
    rootfs_dir = os.path.join(root_dir, "src", "rootfs")
    download_path = os.path.join(root_dir, "src", "publish", "download_pkgs.py")

    
    config_dir = os.path.join(root_dir, "src", "scripts", "configs_group", group_name)
    version = None
    if len(sys.argv) >= 2:
        version = sys.argv[1]

    if version:
        # 如果传入了版本号，调用download.py下载特定版本
        print(f"Downloading version: {version}")
        subprocess.run([sys.executable, download_path, version, system_os, architecture], check=True)

        rootfs_dir = f"/opt/buckyosci/rootfs/{version}/buckyos-{system_os}-{architecture}"
    else:
        # 如果没有传入版本号，认为rootfs下有已经编译好的版本
        print("No version specified, using existing rootfs.")

    # 拷贝到准备好的temp下临时目录
    shutil.copytree(rootfs_dir, temp_root_dir, dirs_exist_ok=True)

    # 拷贝临时身份文件
    print("Copying temporary identity files...")
    shutil.copytree(config_dir, os.path.join(temp_root_dir, "etc"), dirs_exist_ok=True)

    # 启动node-daemon
    print("Starting node-daemon...")
    daemon_env = os.environ.copy()
    daemon_env["BUCKYOS_ROOT"] = temp_root_dir
    node_daemon_path = os.path.join(temp_root_dir, "bin", "node_daemon", f"node_daemon{ext}")
    if platform.system() == "Windows":
        subprocess.Popen([node_daemon_path,"--enable_active"], 
                        env=daemon_env,
                        cwd=os.path.join(temp_root_dir, "bin"),
                        creationflags=subprocess.CREATE_NEW_CONSOLE)
    else:
        subprocess.Popen([node_daemon_path,"--enable_active"], 
                        env=daemon_env,
                        cwd=os.path.join(temp_root_dir, "bin"),
                        stdout=subprocess.DEVNULL, 
                        stderr=subprocess.DEVNULL)

    testcases_output_dir = os.path.join(root_dir, "test_output")
    if os.path.exists(testcases_output_dir):
        shutil.rmtree(testcases_output_dir, ignore_errors=True)

    os.makedirs(testcases_output_dir, exist_ok=True)

    # wait 30 secs for node-daemon to start
    print("Waiting 30 seconds for node-daemon to start...")
    import time
    time.sleep(30)

    print("starting all testcases...")
    test_results = {}
    # 遍历testcases的所有子目录，如果存在test.py测试脚本，则执行它
    testcases_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "testcases")
    for item in os.listdir(testcases_dir):
        item_path = os.path.join(testcases_dir, item)
        if os.path.isdir(item_path):
            test_script = os.path.join(item_path, "test.py")
            if os.path.exists(test_script):
                print(f"Running test script: {test_script}")
                testout_file = open(os.path.join(testcases_output_dir, f"{item}_output.log"), "w")
                result = subprocess.run([sys.executable, test_script, temp_root_dir], cwd=item_path, stdout=testout_file, stderr=testout_file)
                if result.returncode != 0:
                    print(f"Test script {test_script} failed with return code {result.returncode}")
                    test_results[item] = "Failed"
                else:
                    print(f"Test script {test_script} completed successfully.")
                    test_results[item] = "Passed"
            else:
                print(f"No test.py found in {item_path}, skipping...")
        else:
            print(f"{item_path} is not a directory, skipping...")

    print(test_results)

try:
    main()
except Exception as e:
    print(f"Exception occurred during testing: {e}")
    traceback.print_exc()
finally:
    print("Stopping node-daemon...")
    killall_script = os.path.join(temp_root_dir, "bin", "killall.py")
    if os.path.exists(killall_script):
        subprocess.run([sys.executable, killall_script], cwd=os.path.join(temp_root_dir, "bin"))
    print("Cleaning up temporary files...")
    #shutil.rmtree(temp_root_dir, ignore_errors=True)