import sys
import os

current_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.append(current_dir)
import sn
import util
import remote_device


def gen_dev_config_path(sub_path: str):
    return os.path.join(util.CONFIG_BASE, sub_path)

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
    # 获取项目根目录
    project_root = os.path.dirname(os.path.dirname(os.path.dirname(current_dir)))
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

