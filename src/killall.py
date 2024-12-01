
import os
import platform

system = platform.system()
ext = ""
killall_command = "killall"
if system == "Windows":
    ext = ".exe"
    killall_command = "taskkill /F /IM"

def kill_process(name):
    if os.system(f"{killall_command} {name}{ext}") != 0:
        print(f"{name} not running")
    else:
        print(f"{name} killed")

kill_process("node_daemon")
kill_process("scheduler")
kill_process("verify_hub")
kill_process("system_config")
kill_process("cyfs_gateway")


