import subprocess
import platform
import os
import locale
import shlex
import signal

system = platform.system()
ext = ""
if system == "Windows":
    ext = ".exe"

def ensure_directory_accessible(directory_path):
    if not os.path.exists(directory_path):
        os.makedirs(directory_path, exist_ok=True)
    os.system(f"chmod -R 777 {directory_path}")
    
# 获取系统默认编码
def get_system_encoding():
    try:
        return locale.getpreferredencoding()
    except:
        return 'utf-8'
    
def get_user_data_dir(user_id: str) -> str:
    return os.path.join(get_buckyos_root(),"data", user_id)

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
        # 使用tasklist命令检查进程
        if not process_full_path.endswith(".exe"):
            process_full_path = process_full_path + ".exe"
        try:
            process_name = os.path.basename(process_full_path)
            check_args = ["tasklist", "/FI", f"IMAGENAME eq {process_name}", "/FO", "CSV"]
            output = subprocess.check_output(check_args).decode(get_system_encoding(), errors='ignore')
            # 检查输出中是否包含进程名（排除标题行）
            lines = output.strip().split('\n')
            if len(lines) > 1:  # 有数据行
                return True
            return False
        except (subprocess.CalledProcessError, FileNotFoundError):
            # 如果tasklist失败，尝试使用wmic（兼容性）
            try:
                check_args = ["wmic", "process", "where", f"ExecutablePath like '%{process_full_path}%'", "get", "ProcessId", "/format:list"]
                output = subprocess.check_output(check_args).decode(get_system_encoding(), errors='ignore')
                return bool(output.strip())
            except (subprocess.CalledProcessError, FileNotFoundError):
                print(f"Warning: Unable to check process existence on Windows. Both tasklist and wmic failed.")
                return False
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
    # 创建环境变量字典
    env = os.environ.copy()
    if env_vars:
        env.update(env_vars)

    if system == "Windows":
        cmd = f"start /min {run_cmd}"
        creationflags = (
            subprocess.DETACHED_PROCESS
            | subprocess.CREATE_NEW_PROCESS_GROUP
            | subprocess.CREATE_NO_WINDOW
        )
        print(f"will run cmd {cmd} on system {system}")
        subprocess.run(cmd, shell=True, creationflags=creationflags, env=env)
        return None

    # POSIX (macOS/Linux): detach from parent session/process group so parent exit won't affect child.
    args = run_cmd if isinstance(run_cmd, (list, tuple)) else shlex.split(str(run_cmd))
    print(f"will run cmd {args} on system {system}")
    proc = subprocess.Popen(
        list(args),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        preexec_fn=os.setsid,  # new session / new process group
        close_fds=True,
        env=env,
    )
    return proc.pid


def run_and_wait(run_cmd, timeout_secs=None, env_vars=None, cwd=None):
    """
    Run command in foreground and wait for it to exit.

    - run_cmd: str | list[str] | tuple[str]
    - timeout_secs: float | int | None (None means wait forever)
    - env_vars: dict[str,str] | None
    - cwd: str | None

    Returns: (returncode: int, timed_out: bool)
    """
    env = os.environ.copy()
    if env_vars:
        env.update(env_vars)

    args = run_cmd if isinstance(run_cmd, (list, tuple)) else shlex.split(str(run_cmd))
    print(f"will run cmd {args} on system {system}, timeout={timeout_secs}s")

    if system == "Windows":
        # Create a new process group so we can terminate the whole tree on timeout.
        creationflags = subprocess.CREATE_NEW_PROCESS_GROUP
        proc = subprocess.Popen(
            list(args),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            creationflags=creationflags,
            cwd=cwd,
            env=env,
            text=True,
            errors="ignore",
        )
        try:
            out, _ = proc.communicate(timeout=timeout_secs)
            if out:
                print(out, end="" if out.endswith("\n") else "\n")
            return proc.returncode, False
        except subprocess.TimeoutExpired:
            print(f"cmd timeout after {timeout_secs}s, will terminate pid={proc.pid}")
            # Best-effort: kill process tree.
            try:
                subprocess.run(
                    ["taskkill", "/F", "/T", "/PID", str(proc.pid)],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            except Exception as e:
                print(f"taskkill failed: {e}")
            return 124, True

    # POSIX: start in a new session/process group and kill the group on timeout.
    proc = subprocess.Popen(
        list(args),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        preexec_fn=os.setsid,
        close_fds=True,
        cwd=cwd,
        env=env,
        text=True,
        errors="ignore",
    )
    try:
        out, _ = proc.communicate(timeout=timeout_secs)
        if out:
            print(out, end="" if out.endswith("\n") else "\n")
        return proc.returncode, False
    except subprocess.TimeoutExpired:
        print(f"cmd timeout after {timeout_secs}s, will terminate pgid={proc.pid}")
        try:
            os.killpg(proc.pid, signal.SIGTERM)
        except Exception as e:
            print(f"killpg(SIGTERM) failed: {e}")
        try:
            proc.wait(timeout=2)
        except Exception:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except Exception as e:
                print(f"killpg(SIGKILL) failed: {e}")
        return 124, True

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

