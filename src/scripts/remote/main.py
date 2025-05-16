import subprocess
import sys
import os
import yaml  # 新增导入 yaml 模块
import json

import py_src.create_vm as create_vm
import py_src.get_device_info as get_device_info
import install
import start
import remote_device
import py_src.util as util


# 配置文件路径
CONFIG_BASE = os.path.join(os.path.dirname(__file__), "dev_configs")
ENV_CONFIG = os.path.join(CONFIG_BASE, "dev_vm_config.json")
VM_DEVICE_CONFIG = os.path.join(CONFIG_BASE, "device_info.json")


def print_usage():
    print("Usage:")
    print("  ./main.py list                     # list vm device info")
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
    if not os.path.exists(ENV_CONFIG):
        print(f"Config file not found: {ENV_CONFIG}")
        sys.exit(1)
    print(f"Using config file: {ENV_CONFIG}")
    creator = create_vm.VMCreator(ENV_CONFIG)
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
    # check config file

    if not os.path.exists(ENV_CONFIG):
        print(f"Config file not found: {ENV_CONFIG}")
        sys.exit(1)
    print(f"VM Using config file: {ENV_CONFIG}")
  


def active_sn():
    temp_config =os.path.join(CONFIG_BASE, "sn_server/web3_gateway.json.temp")
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
    print("sn config file, db file uploaded")
    # vmsn.scp_put("./dev_configs/bobdev/ood1/node_private_key.pem", "/opt/buckyos/etc/node_private_key.pem")
    # vmsn.scp_put("./dev_configs/bobdev/ood1/start_config.json", "/opt/buckyos/etc/start_config.json")


def active():
    nodeB1 = remote_device.remote_device("nodeB1")
    nodeA2 = remote_device.remote_device("nodeA2")

    nodeB1.run_command("mkdir -p /opt/buckyos/etc")
    nodeB1.scp_put("./dev_configs/bobdev/ood1/node_identity.json", "/opt/buckyos/etc/node_identity.json")
    nodeB1.scp_put("./dev_configs/bobdev/ood1/node_private_key.pem", "/opt/buckyos/etc/node_private_key.pem")

    # start_config 激活流程会生成，没激活直接复制过去
    nodeB1.scp_put("./dev_configs/bobdev/ood1/start_config.json", "/opt/buckyos/etc/start_config.json")

    nodeB1.scp_put("./dev_configs/machine.json", "/opt/buckyos/etc/machine.json")
    # nodeB1.run_command("mkdir -p /opt/buckyos/etc/did_docs")
    # nodeB1.scp_put("./dev_configs/bobdev/test.buckyos.io.zone.json", "/opt/buckyos/etc/did_docs/bob.web3.buckyos.org.doc.json")

    print("nodeB1 config file uploaded")

    # 处理DNS配置
    sn_ip =  util.get_multipass_ip("sn")
    # 要考虑sn_ip是非数组的情况
    print(f"nodeB1 will update DNS for {sn_ip[0]}")


    def update_dns(node, ip):
        # 如果 DNS 行已存在但被注释，这条命令会取消注释并修改值
        node.run_command(f"sudo sed -i 's/#DNS=.*/DNS={sn_ip[0]}/' /etc/systemd/resolved.conf")
        # 如果 DNS 行不存在或已经被取消注释，确保它被正确设置
        node.run_command(f"sudo sed -i 's/DNS=.*/DNS={sn_ip[0]}/' /etc/systemd/resolved.conf")
        node.run_command("sudo systemctl restart systemd-resolved")
    
    update_dns(nodeB1, sn_ip[0])
    print("nodeB1 DNS updated")
    update_dns(nodeA2, sn_ip[0])
    print("nodeA2 DNS updated")


def main():
    # argv[1] 是命令行参数
    if len(sys.argv) < 2:
        print_usage()
        sys.exit(0)

    # parse command
    match sys.argv[1]:
        case "clean":
            clean()
            return
        case "init":
            init()
            return
        case "network":
            check_br()
            return
        case "create":
            # check_br()
            create()
            # generate deviceinfo
            get_device_info.get_device_info(info_path=VM_DEVICE_CONFIG)
            return
        case "deviceinfo":
            get_device_info.get_device_info(info_path=VM_DEVICE_CONFIG)
        case "list":
            all_devices = get_device_info.read_from_config(info_path=VM_DEVICE_CONFIG)
            # print有缩进格式
            print("all devices:")
            for device_id in all_devices:
                print(f"device_id: {device_id}")
                print(f"state: {all_devices[device_id]['state']}")
                print(f"ipv4: {all_devices[device_id]['ipv4']}")
                print(f"release: {all_devices[device_id]['release']}")
                print("")
            # print(all_devices)
        case "install":
            if len(sys.argv) < 3:
                print("Usage: install.py <device_id>")
                print("Usage: install.py --all")
                return
            device_id = sys.argv[2]
            if device_id == "--all":
                all_devices = get_device_info.read_from_config(info_path=VM_DEVICE_CONFIG)
                for device_id in all_devices:
                    print(f"install target device_id: {device_id}")
                    install.install(device_id)
            else:
                print(f"install target device_id: {device_id}")
                install.install(device_id)
        case "active_sn":
            active_sn()
        case "active":
            # active 非sn的ood和node
            active()
        case "start":
            if len(sys.argv) < 3:
                print("Usage: start.py <device_id>")
                return
            device_id = sys.argv[2]
            if device_id == "--all":
                all_devices = get_device_info.read_from_config(info_path=VM_DEVICE_CONFIG)
                for device_id in all_devices:
                    print(f"start target device_id: {device_id}")
                    device = remote_device.remote_device(device_id)
                    start.start_all_apps(device)
            else:
                print(f"start target device_id: {device_id}")
                device = remote_device.remote_device(device_id)
                start.start_all_apps(device)
        case _:
            print("unknown command")
            print("")
            print_usage()
            



if __name__ == "__main__":
    main()