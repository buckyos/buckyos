import os
import tempfile
import sys
import subprocess
import platform
import shutil
src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
if platform.system() == "Windows":
    temp_dir = tempfile.gettempdir()
else:
    temp_dir = "/tmp/"
project_name = "buckyos"
target_dir = os.path.join(temp_dir, "rust_build", project_name)
print(f"Target directory: {target_dir}")
def check_musl_gcc():
    """检查 musl-gcc 是否存在"""
    if shutil.which('musl-gcc') is None:
        print("Error: musl-gcc not found. Please install musl-tools.")
        sys.exit(1)

def check_aarch64_toolchain():
    """检查aarch64交叉编译工具链是否存在"""
    if shutil.which('aarch64-linux-gnu-gcc') is None:
        print("Error: aarch64-linux-gnu-gcc not found. Please install gcc-aarch64-linux-gnu.")
        sys.exit(1)

def clean(target_dir):
    print(f"Cleaning build artifacts at ${target_dir}")
    subprocess.run(["cargo", "clean", "--target-dir", target_dir], check=True, cwd=src_dir)

def build_rust(target_dir, target):
    print(f"Building Rust code,target_dir is {target_dir},target is {target}")
    env = os.environ.copy()
    env["OPENSSL_STATIC"] = "1"
    
    # 为不同目标设置不同的RUSTFLAGS
    if target == "x86_64-unknown-linux-musl":
        env["RUSTFLAGS"] = "-C target-feature=+crt-static --cfg tokio_unstable"
    elif target == "aarch64-unknown-linux-gnu":
        env["RUSTFLAGS"] = "-C target-feature=+crt-static --cfg tokio_unstable"
        # 设置交叉编译工具链
        env["CC_aarch64_unknown_linux_gnu"] = "aarch64-linux-gnu-gcc"
        env["CXX_aarch64_unknown_linux_gnu"] = "aarch64-linux-gnu-g++"
        env["AR_aarch64_unknown_linux_gnu"] = "aarch64-linux-gnu-ar"
        env["CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER"] = "aarch64-linux-gnu-gcc"
    
    subprocess.run(["cargo", "build", "--target", target, "--release", "--target-dir", target_dir], 
                   check=True, 
                   cwd=src_dir, 
                   env=env)

if __name__ == "__main__":
    args = sys.argv[1:]
    if len(args) == 0:
        print("NEED ARGUMENT: clean|amd64|aarch64")
        exit(1)
    if len(args) > 0:
        os.makedirs(target_dir, exist_ok=True)
        if args[0] == "clean":
            clean(target_dir)
        elif args[0] == "amd64":
            # 检查musl-gcc
            check_musl_gcc()
            build_rust(target_dir, "x86_64-unknown-linux-musl")
        elif args[0] == "aarch64":
            # 检查aarch64工具链
            check_aarch64_toolchain()
            build_rust(target_dir, "aarch64-unknown-linux-gnu")
        else:
            print("Invalid argument: clean|amd64|aarch64")
            exit(1)