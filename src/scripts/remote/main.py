import subprocess
import sys
import os
import yaml  # 新增导入 yaml 模块
import json

import py_src.create_vm as create_vm
import py_src.get_device_info as get_device_info
import install
import start
import stop
import remote_device
import py_src.util as util
import py_src.clog as clog
import py_src.clean as clean
import py_src.sn as sn
import py_src.state as state
import py_src.run as run


# 配置文件路径
BASE_DIR = os.path.dirname(os.path.abspath(__file__))
CONFIG_BASE = os.path.join(BASE_DIR, "dev_configs")
ENV_CONFIG = os.path.join(CONFIG_BASE, "dev_vm_config.json")
VM_DEVICE_CONFIG = os.path.join(CONFIG_BASE, "device_info.json")


def print_usage():
    print("Usage:")
    print("  ./main.py clean                    # 清除所有的Multipass实例")
    print("  ./main.py clean --force            # 跳过询问，清除所有的Multipass实例")
    print("  ./main.py init                     # 初始化环境")
    print("  ./main.py network                  # 检查是否存在sn-br，并输入ip，如果不存在会创建一个")
    print("  ./main.py create                   # 创建虚拟机")
    print("  ./main.py install <device_id>      # 安装buckyos")
    print("  ./main.py install --all            # 全部vm，安装buckyos")
    print("  ./main.py active                   # 激活测试身份")
    print("  ./main.py active_sn                # 激活测试sn配置信息")
    print("  ./main.py purge <device_id>        # 清除设备（用户）配置信息")
    print("  ./main.py start_sn                 # 启动sn")
    print("  ./main.py start <device_id>        # 启动buckyos")
    print("  ./main.py start --all              # 全部vm，启动buckyos, 但是不会启动sn")
    print("  ./main.py stop <device_id>         # 停止buckyos")
    print("  ./main.py stop --all               # 全部vm，停止buckyos")
    print("  ./main.py all_in_one               # 一键快速启动配置内的所有vm，包括安装激活启动步骤")
    print("  ./main.py clog                     # 收集node日志")
    print("  ./main.py info                     # list vm device info")



def gen_dev_config_path(sub_path: str):
    return os.path.join(CONFIG_BASE, sub_path)


# create vm by read demo_env.json
def create():
    if not os.path.exists(ENV_CONFIG):
        print(f"Config file not found: {ENV_CONFIG}")
        sys.exit(1)
    print(f"Using config file: {ENV_CONFIG}")
    creator = create_vm.VMCreator(ENV_CONFIG, CONFIG_BASE)
    creator.create_all()


# chekc network bridge
# 检查是否存在 br-sn 网络桥接
def network():
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



def purge():
    if len(sys.argv) < 3:
        print("Usage: main.py purge <device_id>")
        return
    device_id = sys.argv[2]
    if device_id == "sn":
        print("sn no support purge")
        return
    device = remote_device.remote_device("nodeA2")
    device.run_command("sudo rm /opt/buckyos/etc/node_identity.json")
    device.run_command("sudo rm /opt/buckyos/etc/node_private_key.pem")
    device.run_command("sudo rm /opt/buckyos/etc/start_config.json")
    print("purge config ok")



def init(): 
    # check config file
    if not os.path.exists(ENV_CONFIG):
        print(f"Config file not found: {ENV_CONFIG}")
        sys.exit(1)
    print(f"VM Using config file: {ENV_CONFIG}")
  



def active():
    nodeB1 = remote_device.remote_device("nodeB1")
    nodeA2 = remote_device.remote_device("nodeA2")

    print("nodeB1 config file uploading......")
    nodeB1.run_command("mkdir -p /opt/buckyos/etc")
    nodeB1.scp_put(gen_dev_config_path("bobdev/ood1/node_identity.json"), "/opt/buckyos/etc/node_identity.json")
    nodeB1.scp_put(gen_dev_config_path("bobdev/ood1/node_private_key.pem"), "/opt/buckyos/etc/node_private_key.pem")
    # start_config 激活流程会生成，没激活直接复制过去
    nodeB1.scp_put(gen_dev_config_path("bobdev/ood1/start_config.json"), "/opt/buckyos/etc/start_config.json")
    nodeB1.scp_put(gen_dev_config_path("bobdev/cyfs_gateway.json"), "/opt/buckyos/etc/cyfs_gateway.json")
    nodeB1.scp_put(gen_dev_config_path("bobdev/node_gateway.json"), "/opt/buckyos/etc/node_gateway.json")

    nodeB1.scp_put(gen_dev_config_path("machine.json"), "/opt/buckyos/etc/machine.json")
    print("nodeB1 config file uploaded")

    # 处理nodeA2的配置文件
    print("nodeA2 config file uploading......")
    nodeA2.run_command("mkdir -p /opt/buckyos/etc")
    project_root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    rootfs_path = os.path.join(project_root, "rootfs")
    nodeA2.scp_put(os.path.join(rootfs_path, "etc/node_identity.json"), "/opt/buckyos/etc/node_identity.json")
    nodeA2.scp_put(os.path.join(rootfs_path, "etc/node_private_key.pem"), "/opt/buckyos/etc/node_private_key.pem")
    nodeA2.scp_put(os.path.join(rootfs_path, "etc/start_config.json"), "/opt/buckyos/etc/start_config.json")
    nodeA2.scp_put(gen_dev_config_path("machine.json"), "/opt/buckyos/etc/machine.json") # 不能用rootfs下面的machine.json
    print("nodeA2 config file uploaded")




    # 处理DNS配置
    sn_ip =  util.get_multipass_ip("sn")
    # 要考虑sn_ip是非数组的情况
    print(f"sn IP {sn_ip[0]}")

    
    sn.update_node_dns(nodeB1, sn_ip[0])
    sn.update_node_dns(nodeA2, sn_ip[0])



def main():
    # argv[1] 是命令行参数
    if len(sys.argv) < 2:
        print_usage()
        sys.exit(0)

    # parse command
    match sys.argv[1]:
        case "clean":
            clean.cleanInstances()
            return
        case "init":
            init()
            return
        case "network":
            network()
            return
        case "create":
            create()
            # 创建完成后，会生成generate deviceinfo
            get_device_info.get_device_info(info_path=VM_DEVICE_CONFIG)
            return
        case "deviceinfo":
            get_device_info.get_device_info(info_path=VM_DEVICE_CONFIG)
        case "info":
            state.info_device()
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
            sn.active_sn(CONFIG_BASE)
        case "active":
            # active 非sn的ood和node
            active()
        case "purge":
            purge()
        case "start_sn":
            sn.start_sn()
        case "start":
            if len(sys.argv) < 3:
                print("Usage: main.py start <device_id>")
                return
            device_id = sys.argv[2]
            if device_id == "--all":
                all_devices = get_device_info.read_from_config(info_path=VM_DEVICE_CONFIG)
                for device_id in all_devices:
                    # 不在这里启动sn，在start_sn中启动
                    if device_id == "sn":
                        continue
                    print(f"start target device_id: {device_id}")
                    device = remote_device.remote_device(device_id)
                    start.start_all_apps(device)
            else:
                if device_id == "sn":
                    print("use start_sn replace")
                    return
                print(f"start target device_id: {device_id}")
                device = remote_device.remote_device(device_id)
                start.start_all_apps(device)
        case "stop":
            if len(sys.argv) < 3:
                print("Usage: stop.py <device_id>")
                return
            device_id = sys.argv[2]
            if device_id == "--all":
                all_devices = get_device_info.read_from_config(info_path=VM_DEVICE_CONFIG)
                for device_id in all_devices:
                    print(f"stop target device_id: {device_id}")
                    device = remote_device.remote_device(device_id)
                    stop.stop_all_apps(device)
            else:
                print(f"stop target device_id: {device_id}")
                device = remote_device.remote_device(device_id)
                stop.stop_all_apps(device)
        case "clog":
            if len(sys.argv) >= 3:
                device_id = sys.argv[2]
                print("Collecting log for device_id: ", device_id)
                device = remote_device.remote_device(device_id)
                clog.get_device_log(device)
                return
            all_devices = get_device_info.read_from_config(info_path=VM_DEVICE_CONFIG)
            for device_id in all_devices:
                device = remote_device.remote_device(device_id)
                clog.get_device_log(device)
        case "all_in_one":
            run.run()
        case _:
            print("unknown command")
            print("")
            print_usage()
            



if __name__ == "__main__":
    main()