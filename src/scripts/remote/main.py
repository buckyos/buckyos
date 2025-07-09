import subprocess
import sys
import os
import yaml  # 新增导入 yaml 模块
import json

import py_src.create_vm as create_vm
import py_src.get_device_info as get_device_info
import py_src.install as install
import py_src.start as start
import py_src.stop as stop
import py_src.remote_device as remote_device
import py_src.util as util
import py_src.clog as clog
import py_src.clean as clean
import py_src.sn as sn
import py_src.state as state
import py_src.run as run
import py_src.active as active


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
    if not os.path.exists(util.ENV_CONFIG):
        print(f"Config file not found: {util.ENV_CONFIG}")
        sys.exit(1)
    print(f"VM Using config file: {util.ENV_CONFIG}")
  

def main():
    # argv[1] 是命令行参数
    if len(sys.argv) < 2:
        print_usage()
        sys.exit(0)

    # parse command
    match sys.argv[1]:
        case "clean":
            clean.cleanInstances()
        case "init":
            init()
        case "network":
            network()
        case "create":
            create_vm.create()
            # 创建完成后，会生成generate deviceinfo
            get_device_info.get_device_info()
        case "deviceinfo":
            get_device_info.get_device_info()
        case "info":
            state.info_device()
        case "install":
            install.main()
        case "active_sn":
            sn.active_sn(util.CONFIG_BASE)
        case "active":
            # active 非sn的ood和node
            active.active()
        case "purge":
            purge()
        case "start_sn":
            sn.start_sn()
        case "start":
            start.main()
        case "stop":
            stop.main()
        case "clog":
            if len(sys.argv) >= 3:
                device_id = sys.argv[2]
                print("Collecting log for device_id: ", device_id)
                device = remote_device.remote_device(device_id)
                clog.get_device_log(device)
                return
            all_devices = get_device_info.read_from_config(info_path=util.VM_DEVICE_CONFIG)
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