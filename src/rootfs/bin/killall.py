import os
import platform
import subprocess

system = platform.system()
ext = ""
killall_command = "killall"
if system == "Windows":
    ext = ".exe"
    killall_command = "taskkill /F /IM"

def stop_app_container(name):
    # TODO 现在只处理了filebrowser，可能还需要处理后续其他app
    # stop and remove 'devtest-buckyos-filebrowser' container
    if name == 'filebrowser' and system != "Windows":
        result_stop = subprocess.run(['docker', 'stop', 'devtest-buckyos-filebrowser'], capture_output=True, text=True)
        if result_stop.returncode != 0:
            print(f"Failed to stop {name} container: {result_stop.stderr}")
        else:
            print(f"{name} container stopped")
        #result_remove = subprocess.run(['docker', 'rm', 'devtest-buckyos-filebrowser'], capture_output=True, text=True)
        #print(f"{name} container removed")

def kill_process(name):
    if os.system(f"{killall_command} {name}{ext}") != 0:
        print(f"{name} not running")
    else:
        print(f"{name} killed")

def kill_all():
    kill_process("node_daemon")
    kill_process("scheduler")
    kill_process("verify_hub")
    kill_process("system_config")
    kill_process("cyfs_gateway")
    kill_process("filebrowser")
    stop_app_container("filebrowser")
    kill_process("smb_service")
    kill_process("repo_service")

kill_all()