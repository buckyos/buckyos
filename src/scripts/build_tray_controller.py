import os
import sys
import tempfile
import subprocess
import platform

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "../apps/tray-controller")
target_dir = os.path.join(tempfile.gettempdir(), "rust_build", "tray_controller")

def clean(target_dir):
    print(f"Cleaning build artifacts at {target_dir}")
    subprocess.run(["cargo", "clean", "--target-dir", target_dir], check=True, cwd=src_dir)

def build_rust(target_dir, target):
    print(f"Building Rust code, target_dir is {target_dir}, target is {target}")
    env = os.environ.copy()
    env["OPENSSL_STATIC"] = "1"
    env["RUSTFLAGS"] = "-C target-feature=+crt-static --cfg tokio_unstable"
    subprocess.run(["cargo", "build", "--target", target, "--release", "--target-dir", target_dir], 
                    check=True, 
                    cwd=src_dir, 
                    env=env)

def build(target):
    build_rust(target_dir, target)

def prepare_win(auto_win_sdk: bool):
    import winreg

    def is_sdk_installed():
        try:
            # 方式1：检查注册表
            key = winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows Kits\Installed Roots")
            winreg.QueryValueEx(key, "KitsRoot10")  # 检查Windows 10 SDK路径
            return True
        except FileNotFoundError:
            pass
        
        # 方式2：检查默认安装目录
        default_path = r"C:\Program Files (x86)\Windows Kits\10\Include"
        if os.path.exists(default_path):
            return True
            
        return False

    def install_sdk():
        print("Downloading Windows SDK...")
        sdk_url = "https://go.microsoft.com/fwlink/p/?linkid=2120843"
        installer_path = os.path.join(os.environ["RUNNER_TEMP"], "winsdk.exe")
        
        # 下载安装包
        subprocess.run([
            "powershell",
            f"Invoke-WebRequest -Uri '{sdk_url}' -OutFile '{installer_path}'"
        ], check=True)
        
        # 静默安装
        print("Installing Windows SDK...")
        subprocess.run([
            installer_path,
            "/install",
            "/quiet",
            "/norestart",
            "WindowsSDKSigningTools=1",
            "WindowsSDK_10=1"
        ], check=True)
        
        # 清理安装包
        os.remove(installer_path)


    if not is_sdk_installed():
        print("Windows SDK not found, installing...")
        if auto_win_sdk:
            install_sdk()
            print("Installation completed")
        else:
            exit(-1)
    else:
        print("Windows SDK already installed")

if __name__ == "__main__":
    auto_win_sdk = False

    target = ""
    system = platform.system() # Linux / Windows / Darwin
    arch = platform.machine() # x86_64 / AMD64 / arm64 / arm
    if system == "Linux" and (arch == "x86_64" or arch == "AMD64"):
        target = "x86_64-unknown-linux-musl"
    elif system == "Windows" and (arch == "x86_64" or arch == "AMD64"):
        target = "x86_64-pc-windows-msvc"
    elif system == "Darwin" and (arch == "arm64" or arch == "arm"):
        target = "aarch64-apple-darwin"
    
    for arg in sys.argv[1:]:
        if arg == "--auto-win-sdk":
            auto_win_sdk = True
        elif arg.startswith("--target="):
            target = arg.split("=")[1]
        elif arg == "clean":
            clean(target_dir)
            exit(0)

    if system == 'Windows':
        prepare_win(auto_win_sdk)

    os.makedirs(target_dir, exist_ok=True)
    build(target)