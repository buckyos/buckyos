
import os
import sys
import subprocess

try:
    import buckyos_devkit
except ImportError:
    print("buckyos-devkit not found, please install it first")
    print('pip install "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git"')
    sys.exit(1)

if __name__ == "__main__":
    # 调用 buckyos-build 命令，传递所有参数
    print("!!! buckyos depend on cyfs-gateway, MAKE SURE YOU HAVE BUILD IT FIRST!")
    result = subprocess.run(
        ["buckyos-build"] + sys.argv[1:],
        env=os.environ.copy()
    )
    # 使用相同的退出码退出
    if result.returncode != 0:
        print(f"buckyos-build failed with return code {result.returncode}")
        sys.exit(result.returncode)
    
    result = subprocess.run(
        ["buckyos-update"],
        env=os.environ.copy()
    )
    if result.returncode != 0:
        print(f"buckyos-update failed with return code {result.returncode}")
        sys.exit(result.returncode)
    print("buckyos-build and buckyos-update completed successfully")
    sys.exit(0)

