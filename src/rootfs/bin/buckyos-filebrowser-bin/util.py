import subprocess
import platform
import os

system = platform.system()
ext = ""
if system == "Windows":
    ext = ".exe"


def get_app_data_dir(app_id: str,owner_user_id: str) -> str:
    return os.path.join(get_buckyos_root(),"data", owner_user_id, app_id)

def get_app_cache_dir(app_id: str,owner_user_id: str) -> str:
    return os.path.join(get_buckyos_root(),"cache", owner_user_id, app_id)

def get_app_local_cache_dir(app_id: str,owner_user_id: str) -> str:
    return os.path.join(get_buckyos_root(),"tmp", owner_user_id, app_id)

def get_session_token_env_key(app_full_id: str, is_app_service: bool) -> str:
    app_id = app_full_id.upper().replace("-", "_")
    if not is_app_service:
        return f"{app_id}_SESSION_TOKEN"
    else:
        return f"{app_id}_TOKEN"
    
def get_full_appid(app_id: str, owner_user_id: str) -> str:
    return f"{owner_user_id}-{app_id}"



# TODO:process_full_path是目标进程的完整路径
def check_process_exists(process_full_path):
    if system == "Windows":
        # 在Windows上使用wmic来检查进程的完整路径
        check_args = ["wmic", "process", "where", f"ExecutablePath like '%{process_full_path}%'", "get", "ProcessId", "/format:list"]
    else:
        # pgrep 使用 -f 选项可以匹配完整的命令行，包括完整路径
        # 如果 process_full_path 是进程名称，则直接匹配
        # 如果是完整路径，则使用 -f 选项进行模式匹配
        check_args = ["pgrep", "-f", process_full_path]


    try:
        output = subprocess.check_output(check_args).decode()
        #print(f"check_process_exists {process_name} output: {output}")
        return bool(output.strip())  # 如果输出不为空，则进程存在
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
        killall_command = "taskkill /F /IM"

    if os.system(f"{killall_command} {name}{ext}") != 0:
        print(f"{name} not running")
    else:
        print(f"{name} killed")

def nohup_start(run_cmd, env_vars=None):
    cmd = f"nohup {run_cmd} > /dev/null 2>&1 &"
    creationflags = 0
    if system == "Windows":
        cmd = f"start /min {run_cmd}"
        creationflags = subprocess.DETACHED_PROCESS|subprocess.CREATE_NEW_PROCESS_GROUP|subprocess.CREATE_NO_WINDOW
    
    print(f"will run cmd {cmd} on system {system}")
    
    # 创建环境变量字典
    env = os.environ.copy()
    if env_vars:
        env.update(env_vars)
    
    subprocess.run(cmd, shell=True, creationflags=creationflags, env=env)
    # os.system(cmd)

def get_buckyos_root():
    buckyos_root = os.environ.get("BUCKYOS_ROOT")
    if buckyos_root:
        return buckyos_root

    if system == "Windows":
        user_data_dir = os.environ.get("APPDATA")
        if not user_data_dir:
            user_data_dir = os.environ.get("USERPROFILE", ".")
        return os.path.join(user_data_dir, "buckyos")
    else:
        return "/opt/buckyos/"

