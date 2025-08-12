import os
import re
import socket
import subprocess
import time

import pytest
import requests

from remote_node import RemoteNode
from krpc import sn_check_active_code, sn_check_username, regsiter_sn_user, register_domain_ip, query_with_dns

local_path = os.path.realpath(os.path.dirname(__file__))
identity_file = os.path.realpath(os.path.join(os.path.join(local_path, '../remote'), 'dev_configs/ssh/id_rsa'))
rootfs = os.path.realpath(os.path.join(local_path, '../../rootfs'))

def _run_command(cmd: str, check=True) -> tuple:
    """执行shell命令"""
    try:
        result = subprocess.run(cmd, shell=True, check=check,
                                stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE,
                                text=True)
        return result.stdout, result.stderr
    except subprocess.CalledProcessError as e:
        print(f"Command failed: {cmd}")
        print(f"Error: {e.stderr}")
        if check:
            raise
        return e.stdout, e.stderr


def vm_exist(vm_name: str) -> bool:
    stdout, _ = _run_command("multipass list", check=False)
    existing_vms = [line.split()[0] for line in stdout.split('\n')[1:] if line.strip()]

    if vm_name in existing_vms:
        return True
    else:
        return False


def get_vm_ips(vm_name: str) -> [str]:
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

        return devices[vm_name]['ipv4']
    except Exception as e:
        print(f"An error occurred: {e}")
        return None


def read_file(file_path: str):
    with open(file_path, 'r', encoding='utf-8') as f:
        return f.read()


def write_file(file_path: str, content: str):
    with open(file_path, 'w', encoding='utf-8') as f:
        f.write(content)


@pytest.fixture(scope='module')
def init_context():
    print("Creating gateway1 VM")
    if not vm_exist('gateway1'):
        _run_command(f'multipass launch --name gateway1 --cpus 1 --memory 1G --disk 10G --cloud-init {local_path}/gateway_vm.yaml')

    ip = get_vm_ips('gateway1')[0]
    gateway1_ip = ip
    remote_node = RemoteNode(ip, identity_file)
    remote_node.run_command('sudo killall cyfs_gateway')
    remote_node.run_command('sudo mkdir /opt/cyfs_gateway')
    remote_node.run_command('sudo mkdir /opt/web3_bridge')
    remote_node.scp_put(os.path.join(rootfs, 'bin/cyfs_gateway/cyfs_gateway'), '/opt/cyfs_gateway/cyfs_gateway')
    remote_node.scp_put(os.path.join(local_path, 'gateway1_key.pem'), '/opt/cyfs_gateway/device_key.pem')
    remote_node.scp_put(os.path.join(local_path, 'web3.buckyos.site.crt'), '/opt/cyfs_gateway/web3.buckyos.site.crt')
    remote_node.scp_put(os.path.join(local_path, 'web3.buckyos.site.key'), '/opt/cyfs_gateway/web3.buckyos.site.key')
    remote_node.scp_put(os.path.join(local_path, 'buckyos.site.crt'), '/opt/cyfs_gateway/buckyos.site.crt')
    remote_node.scp_put(os.path.join(local_path, 'buckyos.site.key'), '/opt/cyfs_gateway/buckyos.site.key')
    remote_node.scp_put(os.path.join(local_path, 'web3.buckyos.xx.crt'), '/opt/cyfs_gateway/web3.buckyos.xx.crt')
    remote_node.scp_put(os.path.join(local_path, 'web3.buckyos.xx.key'), '/opt/cyfs_gateway/web3.buckyos.xx.key')
    remote_node.run_command('sudo kill -9 $(pgrep -f "http_test_server.py")')
    remote_node.scp_put(os.path.join(local_path, 'http_test_server.py'), '/opt/cyfs_gateway/http_test_server.py')
    remote_node.run_command('nohup python3 /opt/cyfs_gateway/http_test_server.py > /dev/null 2>&1 &')
    remote_node.run_command('sudo kill -9 $(pgrep -f "udp_test_server.py")')
    remote_node.scp_put(os.path.join(local_path, 'udp_test_server.py'), '/opt/cyfs_gateway/udp_test_server.py')
    remote_node.run_command('nohup python3 /opt/cyfs_gateway/udp_test_server.py > /dev/null 2>&1 &')
    print("VM gateway1 created successfully")

    print("Creating gateway2 VM")
    if not vm_exist('gateway2'):
        _run_command(f'multipass launch --name gateway2 --cpus 1 --memory 1G --disk 10G --cloud-init {local_path}/gateway_vm.yaml')

    ip = get_vm_ips('gateway2')[0]
    remote_node = RemoteNode(ip, identity_file)
    remote_node.run_command('sudo killall cyfs_gateway')
    remote_node.run_command('sudo mkdir /opt/cyfs_gateway')
    remote_node.run_command('sudo mkdir /opt/web3_bridge')
    remote_node.run_command('sudo mkdir -p /opt/buckyos/etc/did_docs')
    buckyos_doc = read_file(os.path.join(local_path, 'web3.buckyos.io.doc.json.template'))
    buckyos_doc = buckyos_doc.replace('${ip}', gateway1_ip)
    write_file(os.path.join(local_path, 'web3.buckyos.io.doc.json'), buckyos_doc)
    remote_node.scp_put(os.path.join(local_path, 'web3.buckyos.io.doc.json'), '/opt/buckyos/etc/did_docs/web3.buckyos.io.doc.json')
    remote_node.scp_put(os.path.join(rootfs, 'bin/cyfs_gateway/cyfs_gateway'), '/opt/cyfs_gateway/cyfs_gateway')
    remote_node.scp_put(os.path.join(local_path, 'device_key.pem'), '/opt/cyfs_gateway/device_key.pem')

    remote_node.run_command(f"sudo sed -i 's/#DNS=.*/DNS={gateway1_ip}/' /etc/systemd/resolved.conf")
    remote_node.run_command(f"sudo sed -i 's/#Domains=.*/Domains=/' /etc/systemd/resolved.conf")
    remote_node.run_command(f"sudo sed -i 's/#DNSStubListener=.*/DNSStubListener=no/' /etc/systemd/resolved.conf")
    remote_node.run_command("sudo systemctl restart systemd-resolved")
    remote_node.run_command("sudo rm -f /run/systemd/resolve/stub-resolv.conf")
    remote_node.run_command(f"echo 'nameserver {gateway1_ip}' | sudo tee /run/systemd/resolve/stub-resolv.conf")
    remote_node.run_command("sudo chmod 644 /run/systemd/resolve/stub-resolv.conf")

    print("VM gateway2 created successfully")

    yield
    _run_command(f'multipass delete gateway1 --purge')
    _run_command(f'multipass delete gateway2 --purge')


def reset_gateway1(ip: str):
    remote_node = RemoteNode(ip, identity_file)
    remote_node.run_command('sudo killall cyfs_gateway')
    gateway_config = read_file(os.path.join(local_path, 'gateway1.json.template'))
    new_config = gateway_config.replace('${dns_ip}', ip)
    new_config = new_config.replace('${local_ip}', ip)
    write_file(os.path.join(local_path, 'gateway1.json'), new_config)
    remote_node.scp_put(os.path.join(local_path, '../remote/dev_configs/sn_db.sqlite3'), '/opt/web3_bridge/sn_db.sqlite3')
    remote_node.scp_put(os.path.join(local_path, 'gateway1.json'), '/opt/cyfs_gateway/gateway.json')
    remote_node.scp_put(os.path.join(local_path, 'local_dns.toml'), '/opt/cyfs_gateway/local_dns.toml')
    remote_node.run_command('sudo nohup /opt/cyfs_gateway/cyfs_gateway --config_file /opt/cyfs_gateway/gateway.json > /dev/null 2>&1 &')
    time.sleep(2)


def reset_gateway2(ip: str, dest_ip: str):
    remote_node = RemoteNode(ip, identity_file)
    remote_node.run_command('sudo killall cyfs_gateway')
    gateway_config = read_file(os.path.join(local_path, 'gateway2.json.template'))
    new_config = gateway_config.replace('$${dns_ip}', ip)
    new_config = new_config.replace('${dest_ip}', dest_ip)
    write_file(os.path.join(local_path, 'gateway2.json'), new_config)
    remote_node.scp_put(os.path.join(local_path, 'gateway2.json'), '/opt/cyfs_gateway/gateway.json')
    remote_node.scp_put(os.path.join(local_path, 'local_dns.toml'), '/opt/cyfs_gateway/local_dns.toml')
    remote_node.run_command('sudo nohup /opt/cyfs_gateway/cyfs_gateway --config_file /opt/cyfs_gateway/gateway.json > /dev/null 2>&1 &')
    time.sleep(2)

def udp_test(server_ip='127.0.0.1', server_port=8888):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.settimeout(5)

    try:
        message = "test"
        sock.sendto(message.encode('utf-8'), (server_ip, server_port))
        data, _ = sock.recvfrom(1024)
        print(f"收到服务器响应: {data.decode('utf-8')}")
    except Exception as e:
        assert False, f"UDP测试失败: {e}"
    finally:
        sock.close()


def test_forward(init_context):
    init_context
    ips = get_vm_ips('gateway1')
    print(f"gateway1 ips: {ips}")
    reset_gateway1(ips[0])

    resp = requests.get(f"http://{ips[0]}:8080/test")
    assert resp.status_code == 200
    print(resp)
    resp = requests.post(f"http://{ips[0]}:8080/test", json={"test": 1})
    assert resp.status_code == 201
    print(resp)

    udp_test(server_ip=ips[0])
    udp_test(server_ip=ips[0], server_port=5643)

    ips2 = get_vm_ips('gateway2')
    print(f"gateway2 ips: {ips2}")
    reset_gateway2(ips2[0], ips[0])

    udp_test(server_ip=ips2[0], server_port=5643)

    resp = requests.get(f"http://{ips2[0]}:8080/test")
    assert resp.status_code == 200
    print(resp.json())

    resp = requests.get(f"http://{ips2[0]}:8081/test")
    assert resp.status_code == 200
    print(resp.json())



def test_dns(init_context):
    init_context
    ips = get_vm_ips('gateway1')
    print(f"gateway1 ips: {ips}")
    reset_gateway1(ips[0])

    list = query_with_dns('web3.buckyos.io', dns_server=ips[0], record_type="A")
    assert list is not None
    assert len(list) == 1
    assert list[0] == ips[0]
    list = query_with_dns('web3.buckyos.cc', dns_server=ips[0], record_type="A")
    assert list is not None
    assert len(list) == 1
    assert list[0] == ips[0]
    list = query_with_dns('web3.buckyos.ai', dns_server=ips[0], record_type="A")
    assert list is not None
    assert len(list) == 1
    assert list[0] == ips[0]
    list = query_with_dns('web3.buckyos.io', dns_server=ips[0], record_type="AAAA")
    assert list is None
    list = query_with_dns('web3.buckyos.io', dns_server=ips[0], record_type="TXT")
    print(f"web3.buckyos.io: {list}")
    assert list is not None
    list = query_with_dns('web3.buckyos.io', dns_server=ips[0], record_type="SRV")
    assert list is None
    list = query_with_dns('web3.buckyos.io', dns_server=ips[0], record_type="CNAME")
    assert list is None
    list = query_with_dns('web3.buckyos.io', dns_server=ips[0], record_type="MX")
    assert list is None
    list = query_with_dns('web3.buckyos.io', dns_server=ips[0], record_type="NS")
    assert list is None
    list = query_with_dns('web3.buckyos.io', dns_server=ips[0], record_type="PTR")
    assert list is None
    list = query_with_dns('web3.buckyos.io', dns_server=ips[0], record_type="SOA")
    assert list is None
    list = query_with_dns('www.github.com', dns_server=ips[0], record_type="A")
    print(f"www.github.com: {list}")
    assert list is not None
    assert len(list) >= 1
    list = query_with_dns('www.github.com', dns_server=ips[0], record_type="SRV")
    assert list is None
    list = query_with_dns('www.github.com', dns_server=ips[0], record_type="CNAME")
    assert list is None
    list = query_with_dns('www.github.com', dns_server=ips[0], record_type="MX")
    assert list is None
    list = query_with_dns('www.github.com', dns_server=ips[0], record_type="NS")
    assert list is None
    list = query_with_dns('www.github.com', dns_server=ips[0], record_type="PTR")
    assert list is None
    list = query_with_dns('www.github.com', dns_server=ips[0], record_type="SOA")
    assert list is None
    list = query_with_dns('www.github.com', dns_server=ips[0], record_type="A", dns_port=534)
    assert list is None

    list = query_with_dns('www.buckyos.com', dns_server=ips[0], record_type="TXT")
    print(f"www.buckyos.com: {list}")
    assert list is not None
    assert len(list) == 2
    assert list[0] == '"THISISATEST"'

    list = query_with_dns('test.buckyos.com', dns_server=ips[0], record_type="A")
    assert list is not None
    assert len(list) == 1
    assert list[0] == "192.168.1.2"

    list = query_with_dns('test.sub.buckyos.com', dns_server=ips[0], record_type="A")
    assert list is not None
    assert len(list) == 1
    assert list[0] == "192.168.1.3"

    list = query_with_dns('mail.buckyos.com', dns_server=ips[0], record_type="A")
    assert list is not None
    assert len(list) == 1
    assert list[0] == "192.168.1.106"


@pytest.mark.asyncio
async def test_sn(init_context):
    init_context
    ips = get_vm_ips('gateway1')
    print(f"gateway1 ips: {ips}")
    reset_gateway1(ips[0])
    await sn_check_active_code(ips[0], "123456121234", False)
    await sn_check_active_code(ips[0], None, False)
    await sn_check_active_code(ips[0], "22222")
    await sn_check_username(ips[0], "test")
    await regsiter_sn_user(ips[0], None, None, None, None, assert_failed=False)
    await regsiter_sn_user(ips[0], "test", "22222", "dsdfsdf", "sdfsdfsdfg")
    await sn_check_active_code(ips[0], "22222", False)
    await sn_check_username(ips[0], "test", False)


def test_http_server(init_context):
    init_context
    ips = get_vm_ips('gateway1')
    print(f"gateway1 ips: {ips}")
    reset_gateway1(ips[0])
    register_domain_ip("web3.buckyos.ai", ips[0])
    register_domain_ip("web3.buckyos.com", ips[0])
    resp = requests.get(f"http://web3.buckyos.ai/static/gateway.json")
    assert resp.status_code == 200
    resp = requests.get(f"http://web3.buckyos.ai/static/gateway1.json")
    assert resp.status_code == 404
    resp = requests.get(f"http://web3.buckyos.ai/test_upstream")
    assert resp.status_code == 200
    resp = requests.get(f"http://web3.buckyos.ai/test_upstream/test")
    assert resp.status_code == 200
    resp = requests.get(f"http://web3.buckyos.ai/test_upstream_permanent", allow_redirects=False)
    assert resp.status_code == 308
    resp = requests.get(f"http://web3.buckyos.ai/test_upstream_temporary", allow_redirects=False)
    assert resp.status_code == 307
    response = requests.options(f"http://web3.buckyos.ai/static/gateway.json")
    if "Access-Control-Allow-Origin" not in response.headers:
        assert False, "Access-Control-Allow-Origin header not found in response"
    resp = requests.get(f"http://{ips[0]}/test_upstream")
    assert resp.status_code == 200
    resp = requests.get(f"http://{ips[0]}/test_upstream/test")
    assert resp.status_code == 200
    resp = requests.get(f"http://{ips[0]}/test_upstream_permanent", allow_redirects=False)
    assert resp.status_code == 308
    resp = requests.get(f"http://{ips[0]}/test_upstream_temporary", allow_redirects=False)
    assert resp.status_code == 307
    resp = requests.get(f"http://{ips[0]}/_test_upstream/test")
    assert resp.status_code == 404
    resp = requests.get(f"http://web3.buckyos.com/static/gateway.json", headers={
        "Referer": "https://web3.buckyos.com/previous-page",
        "User-Agent": "Mozilla/5.0"
    })
    assert resp.status_code == 200
    response = requests.options(f"http://web3.buckyos.com/static/gateway.json")
    # print(response.headers)
    # if "Access-Control-Allow-Origin" in response.headers:
    #     assert False, "Access-Control-Allow-Origin header in response"
    response = requests.options(f"http://web3.buckyos.ai/test_upstream")
    if "Access-Control-Allow-Origin" not in response.headers:
        assert False, "Access-Control-Allow-Origin header not found in response"


def test_https_server(init_context):
    init_context
    ips = get_vm_ips('gateway1')
    print(f"gateway1 ips: {ips}")
    reset_gateway1(ips[0])
    register_domain_ip("web3.buckyos.site", ips[0])
    register_domain_ip("web3.buckyos.xx", ips[0])
    register_domain_ip("www.buckyos.site", ips[0])
    resp = requests.get(f"https://web3.buckyos.site/static/gateway.json", verify=False)
    assert resp.status_code == 200
    resp = requests.get(f"https://web3.buckyos.xx/static/gateway.json", verify=False)
    assert resp.status_code == 200
    resp = requests.get(f"https://www.buckyos.site/static/gateway.json", verify=False)
    assert resp.status_code == 200
    resp = requests.get(f"http://web3.buckyos.site/test_upstream", verify=False)
    assert resp.status_code == 200
    resp = requests.get(f"https://web3.buckyos.site/test_upstream", verify=False)
    assert resp.status_code == 200
