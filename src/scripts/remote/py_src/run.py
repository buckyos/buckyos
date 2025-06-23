
import subprocess
import sys
import os
import yaml  # 新增导入 yaml 模块
import json

import sn
import active
import start




def run():
    def check_vm_state(name: str):
        result = subprocess.run(['multipass', 'info', name], capture_output=True, text=True)
        if result.returncode == 0:
            info_lines = result.stdout.strip().split('\n')
            state = info_lines[1]
            _, value = state.split(':', 1)
            return value.strip()
        else:
            return ""
    sn_state = check_vm_state('sn')
    nodeA2_state = check_vm_state('nodeA2')
    nodeB1_state = check_vm_state('nodeB1')
    print(f"sn_state:      {sn_state}")
    print(f"nodeA2_state:  {nodeA2_state}")
    print(f"nodeB1_state:  {nodeB1_state}")

    if sn_state == "Running" and nodeA2_state == "Running" and nodeB1_state == "Running":
        # check service start
        print("\nAll VMs are Running")
        sn.active_sn()
        active.active()

        sn.start_sn()

        sys.argv = ['', '', '--all'] 
        start.main()
        # check_active()
        #next step
        #check active
    # else:
        # start

def file_exists(vm: str, file_path: str):
    # 使用test命令检查文件是否存在，再使用head命令读取文件内容
    check_exists = subprocess.run(['multipass', 'exec', vm, 'test', '-f', file_path], capture_output=True)
    if check_exists.returncode != 0:
        return ""
    result = subprocess.run(['multipass', 'exec', vm, 'head', '-n', '1000000', file_path], capture_output=True, text=True)
    # 检查命令执行是否成功
    if result.returncode != 0:
        print(f"获取{file_path}失败")
        sys.exit(1)
    return result.stdout



def check_active():
    file_exists('sn', '/opt/web3_bridge/web3_gateway.json')
    file_exists('sn', "/opt/web3_bridge/sn_db.sqlite3")
    file_exists('sn', "/opt/web3_bridge/device_key.pem")
    print('check sn config file ok')

    

    

        
    