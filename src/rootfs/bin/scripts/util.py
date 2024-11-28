import subprocess
import platform
import os

system = platform.system()
ext = ""
if system == "Windows":
    ext = ".exe"

def check_process_exists(process_name):
    check_args = ["ps", "-A"]
    if system == "Windows":
        check_args = ["tasklist", "/NH", "/FI", f"IMAGENAME eq {process_name}.exe"]
    try:
        output = subprocess.check_output(check_args).decode()
        if process_name in output:
            return True
        else:
            return False
    except subprocess.CalledProcessError:
        return False

def check_port(port) -> bool:
    if port == 0:
        return True
    import socket
    try:
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.settimeout(1)
        sock.connect(('localhost', port))
        sock.close()
        return True
    except Exception as e:
        print(f"An error occurred: {e}")
        return False

def kill_process(name):
    killall_command = "killall"
    if system == "Windows":
        killall_command = "taskkill /IM"

    if os.system(f"{killall_command} {name}{ext}") != 0:
        print(f"{name} not running")
    else:
        print(f"{name} killed")

def nohup_start(run_cmd):
    cmd = f"nohup {run_cmd} > /dev/null 2>&1 &"
    if system == "Windows":
        cmd = f"start /min {run_cmd}"
    print(f"will rum cmd {cmd} on system {system}")
    os.system(cmd)