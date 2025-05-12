import subprocess
import sys
import os
import yaml  # 新增导入 yaml 模块

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


def init(): 
    # 检查是否存在 ~/.buckyos_dev 目录
    home_dir = os.path.expanduser("~")
    config_dir = os.path.join(home_dir, ".buckyos_dev")
    if not os.path.exists(config_dir):
        os.makedirs(config_dir)
        print(f"Created directory: {config_dir}")
    else:
        print(f"Directory already exists: {config_dir}")
    # cp demo_env.json to ~/.buckyos_dev/env_config.json
    demo_env = "demo_env.json"
    env_config = os.path.join(config_dir, "env_config.json")
    if not os.path.exists(env_config):
        os.link(demo_env, env_config)
        print(f"Created link: {env_config} -> {demo_env}")
    else:
        print(f"Link already exists: {env_config}")

    # 创建密钥对
    key_path = os.path.join(config_dir, "id_rsa")
    if not os.path.exists(key_path):
        try:
            subprocess.run(
                f"ssh-keygen -t rsa -b 4096 -f {key_path} -N ''",
                shell=True,
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL
            )
            print(f"Generated SSH key pair at {key_path}")
        except subprocess.CalledProcessError:
            print("Failed to generate SSH key pair")
            return

    # 读取公钥内容
    with open(f"{key_path}.pub", 'r') as f:
        public_key = f.read().strip()

    # 修改 vm_init.yaml
    vm_init_path = "vm_init.yaml"
    with open(vm_init_path, 'r') as f:
        vm_config = yaml.safe_load(f)

    # 确保 ssh_authorized_keys 存在
    if 'users' in vm_config:
        for user in vm_config['users']:
            if user.get('name') == 'root':
                if 'ssh_authorized_keys' not in user:
                    user['ssh_authorized_keys'] = []
                user['ssh_authorized_keys'].append(public_key)
                break

    # 写回 vm_init.yaml
    with open(vm_init_path, 'w') as f:
        yaml.dump(vm_config, f, default_flow_style=False)
    print("Updated vm_init.yaml with new public key")

def main():
    # argv[1] 是命令行参数
    if len(sys.argv) < 2:
        print_usage()
        return
    elif sys.argv[1] == "clean":
        clean()
        return
    elif sys.argv[1] == "init":
        init()
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
    elif sys.argv[1] == "deviceinfo":
        config_path = os.path.expanduser('~/.buckyos_dev/device_info.json')
        get_device_info.get_device_info(info_path=config_path)
    else:
        print_usage()
        return



if __name__ == "__main__":
    main()