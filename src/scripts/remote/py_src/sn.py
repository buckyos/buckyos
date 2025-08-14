import subprocess
import sys
import os
import yaml  # 新增导入 yaml 模块
import json
import util


current_dir = os.path.dirname(os.path.abspath(__file__))
# sys.path.append(current_dir)

parent_dir = os.path.dirname(current_dir)
sys.path.append(parent_dir)
import remote_device
import start




def active_sn():
    base_dir = util.CONFIG_BASE
    temp_config =os.path.join(base_dir, "sn_server/web3_gateway.json.temp")
    sn_ip =  util.get_multipass_ip("sn")
    print(f"sn vm ip: {sn_ip}")

    # 读取sn配置文件模板，修改ip字段，生成配置文件
    with open(temp_config, 'r') as f:
        config = json.load(f)
        config["inner_services"]["main_sn"]["ip"] = sn_ip[0]
        # fix
        config["includes"] = []
        with open(f"{base_dir}/sn_server/web3_gateway.json", 'w') as f:
            json.dump(config, f, indent=4)
        print("generate web3_gateway.json")
        # print(config["inner_services"]["main_sn"]["ip"])

    vmsn = remote_device.remote_device("sn")
    vmsn.run_command("sudo mkdir -p /opt/web3_bridge")
    vmsn.scp_put(f"{base_dir}/sn_server/web3_gateway.json", "/opt/web3_bridge/web3_gateway.json")
    vmsn.scp_put(f"{base_dir}/sn_db.sqlite3", "/opt/web3_bridge/sn_db.sqlite3")
    vmsn.scp_put(f"{base_dir}/sn_server/device_key.pem", "/opt/web3_bridge/device_key.pem")
    vmsn.scp_put(f"{base_dir}/sn_server/resolved.conf", "/opt/web3_bridge/resolved.conf")
    vmsn.scp_put(f"{base_dir}/sn_server/web3_gateway.service", "/opt/web3_bridge/web3_gateway.service")
    print("sn config file, db file uploaded")

    print("disable dnsstub")
    # 不能用 stop systemd-resolved的方式
    # dns_provider.rs里面的
    # `resolver = TokioAsyncResolver::tokio_from_system_conf()` 需要读取 /etc/resolver文件
    vmsn.run_command("sudo cp /opt/web3_bridge/resolved.conf /etc/systemd/resolved.conf")
    vmsn.run_command("sudo systemctl restart systemd-resolved")
    vmsn.run_command("sudo rm /etc/resolv.conf")
    vmsn.run_command("sudo ln -s /run/systemd/resolve/resolv.conf /etc/resolv.conf")

    vmsn.run_command("sudo cp /opt/web3_bridge/web3_gateway.service /etc/systemd/system/web3_gateway.service")
    vmsn.run_command("sudo systemctl daemon-reload")


# sudo mkdir /etc/systemd/resolved.conf.d | echo -e '[Resolve]\nDNSStubListener=no' | sudo tee /etc/systemd/resolved.conf.d/disable-dnsstub.conf

def start_sn():
    device_id = "sn"
    print(f"start target device_id: {device_id}")
    device = remote_device.remote_device(device_id)

    # sn_ip =  util.get_multipass_ip("sn")
    # 要考虑sn_ip是非数组的情况
    # print(f"SN IP {sn_ip[0]}")
    # update_node_dns(device, sn_ip[0])
    # device.run_command("sudo systemctl stop systemd-resolved")
    # device.run_command("sudo systemctl disable systemd-resolved")
    device.run_command("sudo systemctl start web3_gateway")



def update_node_dns(node, ip):
    # 如果 DNS 行已存在但被注释，这条命令会取消注释并修改值
    node.run_command(f"sudo sed -i 's/#DNS=.*/DNS={ip}/' /etc/systemd/resolved.conf")
    # 如果 DNS 行不存在或已经被取消注释，确保它被正确设置
    node.run_command(f"sudo sed -i 's/DNS=.*/DNS={ip}/' /etc/systemd/resolved.conf")
    # node.run_command("sudo systemctl restart systemd-resolved")

    # 这node_daemon和buckycli 启动时候的DNS，只读/etc/resolv.conf, 只能硬改这文件了
    node.run_command(f"sudo sed -i 's/#DNSStubListener=.*/DNSStubListener=no/' /etc/systemd/resolved.conf")
    node.run_command("sudo systemctl restart systemd-resolved")

    # 重启systemd-resolved之后，
    # 把 /etc/resolv.conf中，以.1 结尾的 IP地址的行注释掉
    # 这个地址是，Multipass虚拟网络的默认网关及DNS中继地址，会导致DNS解析错误
    node.run_command(r"sudo sed -ri '/^nameserver [0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.1$/ s/^/# /' /etc/resolv.conf")

    # sudo resolvectl dns ens3 ""
    # sudo resolvectl dns ens4 ""
    # sudo resolvectl flush-caches
    # resolvectl status

    # node.run_command("sudo ln -sf /run/systemd/resolve/resolv.conf /etc/resolv.conf")
    print(f"device DNS updated nameserver update to {ip}")