
import subprocess
import re
import os
import yaml


# 配置文件路径
BASE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CONFIG_BASE = os.path.join(BASE_DIR, "dev_configs")
ENV_CONFIG = os.path.join(CONFIG_BASE, "dev_vm_config.json")
VM_DEVICE_CONFIG = os.path.join(CONFIG_BASE, "device_info.json")
id_rsa_path = os.path.join(CONFIG_BASE, "ssh/id_rsa")



def get_multipass_ip(instance_name):
    try:
        # 执行 multipass info 命令并捕获输出
        result = subprocess.run(
            ["multipass", "info", instance_name],
            capture_output=True,
            text=True,
            check=True  # 检查命令是否执行成功
        )
        
        # 匹配 IPv4 地址（包含多行的情况）
        ip_pattern = r"IPv4:\s+((?:\d+\.\d+\.\d+\.\d+\s*)+)"
        match = re.search(ip_pattern, result.stdout)
        
        if match:
            # 提取所有 IPv4 地址并整理为列表
            ips = [ip.strip() for ip in match.group(1).split()]
            return ips
        else:
            return "未找到 IPv4 地址"
            
    except subprocess.CalledProcessError as e:
        return f"错误：实例 '{instance_name}' 不存在或未运行"
    except Exception as e:
        return f"未知错误：{str(e)}"


# @deprecated
# 创建vm的ssh密钥，密钥配置已经放在dev_configs目录下。
# 正常不需要调用这里
def create_key_pari():
    home_dir = os.path.expanduser("~")
    config_dir = os.path.join(home_dir, ".buckyos_dev")

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