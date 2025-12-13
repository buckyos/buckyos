
import os
import sys
import subprocess


if __name__ == "__main__":
    # 调用 buckyos-build 命令，传递所有参数
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

