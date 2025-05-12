import subprocess
import sys
import os

import create_vm
import get_device_info

def print_usage():
    print("Usage:")
    print("  ./main.py create    # 创建虚拟机")
    print("  ./main.py clean    # 清除所有的Multipass实例")
    print("  ./main.py check    # 检查是否存在网络桥接")



# create vm by read demo_env.json
def create():
    config_file = "demo_env.json"
    if not os.path.exists(config_file):
        print(f"Config file not found: {config_file}")
        sys.exit(1)
    print(f"Using config file: {config_file}")
    creator = create_vm.VMCreator(config_file)
    creator.create_all()


# chekc network bridge
# 检查是否存在 br0 网络桥接
def check_br():
    try:
        # 调用 ip 命令检查网络桥接      
        result = subprocess.run(['ip', 'link', 'show', 'br0'], capture_output=True, text=True)
        if result.returncode == 0:
            print("网络桥接 br0 已存在。")
        else:
            print("网络桥接 br0 不存在。")
    except FileNotFoundError:
        print("未找到 ip 命令，请检查是否已安装。")




def clean():
    try:
        # 调用 multipass 执行清除操作
        subprocess.run(['multipass', 'delete', '--all', '--purge'], check=True)
        print("Multipass 实例已成功清除。")
    except subprocess.CalledProcessError as e:
        print(f"清除 Multipass 实例时出错: {e}")
    except FileNotFoundError:
        print("未找到 multipass 命令，请检查是否已安装。")



def main():
    # argv[1] 是命令行参数
    if len(sys.argv) < 2:
        print_usage()
        return
    elif sys.argv[1] == "clean":
        clean()
        return
    elif sys.argv[1] == "check":
        # TODO create brige if not exists
        check_br()
        return
    elif sys.argv[1] == "create":
        check_br()
        create()
        # generate deviceinfo
        get_device_info.get_device_info()
        return
    else:
        print_usage()
        return



if __name__ == "__main__":
    main()