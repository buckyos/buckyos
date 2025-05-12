import json
import subprocess
import re

def get_device_info(info_path = "device_info.json"):
    # 运行multipass list命令获取设备信息
    try:
        result = subprocess.run(['multipass', 'list'], capture_output=True, text=True)
        output = result.stdout
        
        # 解析输出内容
        devices = {}  # 改用字典而不是列表
        lines = output.strip().split('\n')[1:]  # 跳过标题行
        curr_device_name = None
        for line in lines:
            if line.strip():
                # 使用正则表达式更智能地分割行内容
                parts = re.split(r'\s{2,}', line.strip())
                if len(parts) == 1:
                    ipv4_addresses = parts[0]
                    devices[curr_device_name]['ipv4'].append(ipv4_addresses)
                    continue

                if len(parts) >= 4:
                    device_name = parts[0]
                    curr_device_name = device_name
                    state = parts[1]
                    if state != "Deleted":
                        ipv4_addresses = parts[2]
                        devices[device_name] = {
                            'state': state,
                            'ipv4': [ipv4_addresses],
                            'release': parts[3]
                        }

                    
        
        # 保存到JSON文件
        with open(info_path, 'w', encoding='utf-8') as f:
            json.dump(devices, f, indent=4, ensure_ascii=False)
            
        print(f"Devices information has been successfully saved in {info_path}")
        return devices
        
    except subprocess.CalledProcessError as e:
        print(f"Failed to execute multipass list command: {e}")
        return None
    except Exception as e:
        print(f"An error occurred: {e}")
        return None

if __name__ == '__main__':
    get_device_info()
