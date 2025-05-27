import get_device_info
import subprocess





def info_device():
    # multipass exec sn -- bash -c "ps -ef | grep -v bash | grep -v grep | grep web3_gateway"
    subprocess.run(["multipass", "exec", "sn", "--", "bash", "-c", "ps -ef | grep -v bash | grep -v grep | grep web3_gateway"])




    # all_devices = get_device_info.read_from_config(info_path=VM_DEVICE_CONFIG)
    # # print有缩进格式
    # print("all devices:")
    # for device_id in all_devices:
    #     print(f"device_id: {device_id}")
    #     print(f"state: {all_devices[device_id]['state']}")
    #     print(f"ipv4: {all_devices[device_id]['ipv4']}")
    #     print(f"release: {all_devices[device_id]['release']}")
    #     print("")