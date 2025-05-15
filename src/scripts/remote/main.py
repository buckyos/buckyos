import subprocess
import sys
import os
import yaml  # 新增导入 yaml 模块
import json

import create_vm
import get_device_info
import install
import start
import remote_device
import py_src.util as util



def print_usage():
    print("Usage:")
    print("  ./main.py clean                    # 清除所有的Multipass实例")
    print("  ./main.py init                     # 初始化环境")
    print("  ./main.py network                  # 检查是否存在sn-br，并输入ip，如果不存在会创建一个")
    print("  ./main.py create                   # 创建虚拟机")
    print("  ./main.py install <device_id>      # 安装buckyos")
    print("  ./main.py install --all            # 全部vm，安装buckyos")
    print("  ./main.py active                   # 激活测试身份")
    print("  ./main.py active_sn                 # 激活测试sn配置信息")
    print("  ./main.py start <device_id>        # 启动buckyos")
    print("  ./main.py start --all              # 全部vm，启动buckyos")




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
# 检查是否存在 br-sn 网络桥接
def check_br():
    try:
        # 调用 ip 命令检查网络桥接      
        result = subprocess.run(['ip', 'link', 'show', 'br-sn'], capture_output=True, text=True)
        if result.returncode == 0:
            # ip = subprocess.run("ip -4 addr show dev br-sn | grep -oP 'inet \K[\d.]+'", shell=True, capture_output=True, text=True)
            print(f"网络桥接 br-sn 已存在")
        else:
            print("网络桥接 br-sn 不存在。")
            print("正在创建网络桥接 br-sn...")
            # sudo ip link add br-sn type bridge
            # sudo ip link set br-sn up
            # sudo ip addr add 10.10.10.1/24 dev br-sn'
            subprocess.run(["sudo", "ip", "link", "add", "br-sn", "type", "bridge"])
            subprocess.run(["sudo", "ip", "link", "set", "br-sn", "up"])
            subprocess.run(["sudo", "ip", "addr", "add", "10.10.10.1/24", "dev", "br-sn"])
            print("网络桥接 br-sn 创建完成。")
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
    # 每次覆盖更新env_config.json
    if os.path.exists(env_config):
        os.remove(env_config) 
    os.link(demo_env, env_config)
    print(f"Created link: {env_config} -> {demo_env}")
    # else:
    #     print(f"Link already exists: {env_config}")

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

    need_update = False
    # 确保 ssh_authorized_keys 存在
    if 'users' in vm_config:
        for user in vm_config['users']:
            if user.get('name') == 'root':
                if 'ssh_authorized_keys' not in user:
                    user['ssh_authorized_keys'] = []
                # 检查是否已经存在
                if public_key in user['ssh_authorized_keys']:
                    print("Public key already exists in vm_init.yaml")
                    return
                user['ssh_authorized_keys'].append(public_key)
                need_update = True
                break

    if need_update:
        # 写回 vm_init.yaml
        with open(vm_init_path, 'w') as f:
            yaml.dump(vm_config, f, default_flow_style=False)
        print("Updated vm_init.yaml with new public key")


def active_sn():
    temp_config = "./dev_configs/sn_server/web3_gateway.json.temp"
    sn_ip =  util.get_multipass_ip("sn")
    print(f"sn vm ip: {sn_ip}")

    # 读取sn配置文件模板，修改ip字段，生成配置文件
    with open(temp_config, 'r') as f:
        config = json.load(f)
        config["inner_services"]["main_sn"]["ip"] = sn_ip[0]
        # fix
        config["includes"] = []
        with open("./dev_configs/sn_server/web3_gateway.json", 'w') as f:
            json.dump(config, f, indent=4)
        print("generate web3_gateway.json")
        # print(config["inner_services"]["main_sn"]["ip"])

    vmsn = remote_device.remote_device("sn")
    vmsn.scp_put("./dev_configs/sn_server/web3_gateway.json", "/opt/web3_bridge")
    vmsn.scp_put("./dev_configs/sn_db.sqlite3", "/opt/web3_bridge")
    vmsn.scp_put("./dev_configs/sn_server/device_key.pem", "/opt/web3_bridge")
    print("sn config file uploaded")
    # vmsn.scp_put("./dev_configs/bobdev/ood1/node_private_key.pem", "/opt/buckyos/etc/node_private_key.pem")
    # vmsn.scp_put("./dev_configs/bobdev/ood1/start_config.json", "/opt/buckyos/etc/start_config.json")



def main():
    config_path = os.path.expanduser('~/.buckyos_dev/device_info.json')

    # argv[1] 是命令行参数
    if len(sys.argv) < 2:
        print_usage()
        return
    elif sys.argv[1] == "clean":
        clean()
        return
    elif sys.argv[1] == "init":
        init()
    elif sys.argv[1] == "network":
        check_br()
        return
    elif sys.argv[1] == "create":
        # check_br()
        create()
        # generate deviceinfo
        get_device_info.get_device_info(info_path=config_path)
        return
    elif sys.argv[1] == "deviceinfo":
        get_device_info.get_device_info(info_path=config_path)
    elif sys.argv[1] == "install":
        if len(sys.argv) < 3:
            print("Usage: install.py <device_id>")
            print("Usage: install.py --all")
            return
        device_id = sys.argv[2]
        if device_id == "--all":
            # 遍历所有设备
            with open(config_path, 'r') as f:
                g_all_devices = json.load(f)
                for device_id in g_all_devices:
                    print(f"install target device_id: {device_id}")
                    install.install(device_id)
            return


        print(f"install target device_id: {device_id}")
        install.install(device_id)
    elif sys.argv[1] == "active_sn":
        active_sn()
    elif sys.argv[1] == "active":
        nodeB1 = remote_device.remote_device("nodeB1")
        nodeB1.run_command("mkdir -p /opt/buckyos/etc")
        nodeB1.scp_put("./dev_configs/bobdev/ood1/node_identity.json", "/opt/buckyos/etc/node_identity.json")
        nodeB1.scp_put("./dev_configs/bobdev/ood1/node_private_key.pem", "/opt/buckyos/etc/node_private_key.pem")

        # start_config 激活流程会生成，没激活直接复制过去
        nodeB1.scp_put("./dev_configs/bobdev/ood1/start_config.json", "/opt/buckyos/etc/start_config.json")
        
        # nodeB1.run_command("mkdir -p /opt/buckyos/etc/did_docs")
        # nodeB1.scp_put("./dev_configs/bobdev/test.buckyos.io.zone.json", "/opt/buckyos/etc/did_docs/bob.web3.buckyos.org.doc.json")

        print("nodeB1 config file uploaded")

    elif sys.argv[1] == "start":
        if len(sys.argv) < 3:
            print("Usage: start.py <device_id>")
            return

        device_id = sys.argv[2]
        if device_id == "--all":
            # 遍历所有设备
            with open(config_path, 'r') as f:
                g_all_devices = json.load(f)
                for device_id in g_all_devices:
                    print(f"start target device_id: {device_id}")
                    device = remote_device.remote_device(device_id)
                    start.start_all_apps(device)
            return
        print(f"start target device_id: {device_id}")
        device = remote_device.remote_device(device_id)
        start.start_all_apps(device)

    else:
        print_usage()
        return



if __name__ == "__main__":
    main()