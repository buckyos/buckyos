import ast
import base64
import hashlib
import json
import os
import subprocess
import sys
import time

import pytest

from krpc import kRPCClient, regsiter_sn_user, query_with_dns

local_path = os.path.realpath(os.path.dirname(__file__))

sys.path.append(os.path.realpath(os.path.join(local_path, '../remote/py_src')))
from remote_device import remote_device

@pytest.fixture(scope='module')
def init_context():
    subprocess.run(['python', os.path.join(local_path, '../remote/main.py'), 'init'], check=True)
    subprocess.run(['python', os.path.join(local_path, '../remote/main.py'), 'network'], check=True)
    subprocess.run(['python', os.path.join(local_path, '../remote/main.py'), 'create'], check=True)
    subprocess.run(['python', os.path.join(local_path, '../remote/main.py'), 'install', '--all'], check=True)
    subprocess.run(['python', os.path.join(local_path, '../remote/main.py'), 'active_sn'], check=True)
    subprocess.run(['python', os.path.join(local_path, '../remote/main.py'), 'start_sn'], check=True)
    subprocess.run(['python', os.path.join(local_path, '../remote/main.py'), 'active'], check=True)
    subprocess.run(['python', os.path.join(local_path, '../remote/main.py'), 'start', '--all'], check=True)
    subprocess.run(['python', os.path.join(local_path, '../remote/main.py'), 'deviceinfo'], check=True)
    with open(os.path.join(local_path, '../remote/dev_configs/device_info.json'), 'r') as f:
        resp = json.load(f)
    yield resp
    # subprocess.run(['python', os.path.join(local_path, '../remote/main.py'), 'clean', '--force'], check=True)


def parse_node_info(info: str):
    lines = info.split('\n')
    node_list = []
    cur_device = {}
    for line in lines:
        if line.strip():
            if line.startswith('device_id'):
                key, value = line.split(':', 1)
                cur_device[key.strip()] = value.strip()
            if line.startswith('ipv4'):
                key, value = line.split(':', 1)
                ips = ast.literal_eval(value.strip())
                cur_device[key.strip()] = ips
                node_list.append(cur_device.copy())
    return node_list

async def generate_zone_boot_config_jwt(server: str, sn: str, owner_private_key: str) -> str:
    rpc = kRPCClient(f'http://{server}:3180/kapi/active')
    now = int(time.time())

    if sn == "":
        zone_boot_config = {
            "oods": ["ood1"],
            "exp": now + 3600*24*365*10,
            "iat": now,
        }
    else:
        zone_boot_config = {
            "oods": ["ood1"],
            "sn": sn,
            "exp": now + 3600*24*365*10,
            "iat": now,
        }

    zone_boot_config_str = json.dumps(zone_boot_config)
    try:
        resp = await rpc.call('generate_zone_boot_config', {
            "zone_boot_config": zone_boot_config_str,
            "private_key": owner_private_key,
        })
    except Exception as e:
        assert False, f"generate_zone_boot_config failed: {e}"

    return resp['zone_boot_config_jwt']


async def generate_key_pair(server: str) -> (str, str):
    try:
        rpc = kRPCClient(f'http://{server}:3180/kapi/active')
        resp = await rpc.call('generate_key_pair', {})
    except Exception as e:
        assert False, f"generate_key_pair failed: {e}"

    public_key = resp['public_key']
    private_key = resp['private_key']
    return public_key, private_key


async def do_active(server: str, req: dict, check_success: bool = True):
    try:
        rpc = kRPCClient(f'http://{server}:3180/kapi/active')
        resp = await rpc.call('do_active', req)
    except Exception as e:
        if check_success:
            assert False, f"do_active failed: {e}"
        return

    if not check_success:
        assert False, f"do_active should failed"

async def get_device_info(server: str) -> dict:
    try:
        rpc = kRPCClient(f'http://{server}:3180/kapi/active')
        resp = await rpc.call('get_device_info', {})
    except Exception as e:
        assert False, f"generate_key_pair failed: {e}"
    return resp



def hash_password(username: str, password: str, nonce: int = None) -> str:
    # First hash: password + username + ".buckyos"
    sha256 = hashlib.sha256()
    combined = password + username + ".buckyos"
    sha256.update(combined.encode('utf-8'))
    org_password_hash_bytes = sha256.digest()
    org_password_hash_str = base64.b64encode(org_password_hash_bytes).decode('utf-8')

    if nonce is None:
        return org_password_hash_str

    # Second hash: org_password_hash_str + nonce as string
    sha256 = hashlib.sha256()
    salt = org_password_hash_str + str(nonce)
    sha256.update(salt.encode('utf-8'))
    result_bytes = sha256.digest()
    result = base64.b64encode(result_bytes).decode('utf-8')

    return result

def reset_active(node: str):
    device = remote_device(node)
    device.run_command('sudo python3 /opt/buckyos/bin/killall.py')
    device.run_command('sudo rm -f /opt/buckyos/etc/node_identity.json')
    device.run_command('sudo rm -f /opt/buckyos/etc/node_private_key.pem')
    device.run_command('sudo nohup /opt/buckyos/bin/node_daemon/node_daemon --enable_active > /dev/null 2>&1 &')
    time.sleep(3)


def read_file(file_path: str):
    with open(file_path, 'r', encoding='utf-8') as f:
        return f.read()


def write_file(file_path: str, content: str):
    with open(file_path, 'w', encoding='utf-8') as f:
        f.write(content)


def reset_sn_server(node: str, node_ip: str, config: str = "test"):
    device = remote_device(node)
    device.run_command('sudo python3 /opt/web3_bridge/stop.py')
    gateway_config = read_file(os.path.join(local_path, 'sn.json.template'))
    new_config = gateway_config.replace('${sn_ip}', node_ip)
    write_file(os.path.join(local_path, 'sn.json'), new_config)
    device.scp_put(os.path.join(local_path, 'sn.json'), '/opt/web3_bridge/web3_gateway.json')

    gateway_config = read_file(os.path.join(local_path, 'zone_dns.toml.template'))
    new_config = gateway_config.replace('${config}', f'DID={config};')
    write_file(os.path.join(local_path, 'zone_dns.toml'), new_config)
    device.scp_put(os.path.join(local_path, 'zone_dns.toml'), '/opt/web3_bridge/zone_dns.toml')

    device.scp_put(os.path.join(local_path, '../remote/dev_configs/sn_db.sqlite3'), '/opt/web3_bridge/sn_db.sqlite3')
    device.run_command('sudo python3 /opt/web3_bridge/start.py')
    time.sleep(5)


def kill_sn_server(node: str):
    device = remote_device(node)
    device.run_command('sudo python3 /opt/web3_bridge/stop.py')
    time.sleep(1)


@pytest.mark.asyncio
async def test_active_no_sn(init_context):
    nodes = init_context

    sn_ip = None
    for node_id, node in nodes.items():
        if node_id == 'sn':
            sn_ip = node.get('ipv4')[0]
            break

    if sn_ip is None:
        assert False, "sn node not found"


    for node_id, node in nodes.items():
        if node_id == 'nodeA2':
            reset_active('nodeA2')
            ip = node.get('ipv4')[0]
            owner_public_key, owner_private_key = await generate_key_pair(ip)
            public_key, private_key = await generate_key_pair(ip)

            zone_boot_config_jwt = await generate_zone_boot_config_jwt(ip, "", owner_private_key)
            print(zone_boot_config_jwt)

            reset_sn_server("sn", sn_ip, zone_boot_config_jwt)

            zone_name = "buckyos.test.com"
            req = {
                "sn_url": "",
                "sn_host": "",
                "guest_access": True,
                "admin_password_hash": hash_password("test", "testtest"),
            }
            await do_active(ip, req, False)

            req = {
                "user_name": "test",
                "zone_name": zone_name,
                "sn_url": "",
                "sn_host": "",
                "gateway_type": "PortForward",
                "public_key": "sdfsdfasdf",
                "private_key": "sdfsdfasdf",
                "device_public_key": "sdfsdfasdf",
                "device_private_key": "sdfsdfasdf",
                "guest_access": True,
                "admin_password_hash": hash_password("test", "testtest"),
            }
            await do_active(ip, req, False)

            req = {
                "user_name": "test",
                "zone_name": zone_name,
                "sn_url": "",
                "sn_host": "",
                "gateway_type": "PortForward",
                "public_key": owner_public_key,
                "private_key": owner_private_key,
                "device_public_key": public_key,
                "device_private_key": private_key,
                "guest_access": True,
                "admin_password_hash": hash_password("test", "testtest"),
            }
            await do_active(ip, req)
            time.sleep(3)

            device = remote_device("nodeA2")
            device.run_command('sudo nohup /opt/buckyos/bin/node_daemon/node_daemon --enable_active > /dev/null 2>&1 &')
            time.sleep(10)


@pytest.mark.asyncio
async def test_active_no_sn2(init_context):
    nodes = init_context

    sn_ip = None
    for node_id, node in nodes.items():
        if node_id == 'sn':
            sn_ip = node.get('ipv4')[0]
            break

    if sn_ip is None:
        assert False, "sn node not found"


    for node_id, node in nodes.items():
        if node_id == 'nodeA2':
            reset_active('nodeA2')
            ip = node.get('ipv4')[0]
            owner_public_key, owner_private_key = await generate_key_pair(ip)
            public_key, private_key = await generate_key_pair(ip)

            zone_boot_config_jwt = await generate_zone_boot_config_jwt(ip, "web3.buckyos.io", owner_private_key)
            print(zone_boot_config_jwt)

            reset_sn_server("sn", sn_ip, zone_boot_config_jwt)

            zone_name = "buckyos.test.com"

            req = {
                "user_name": "test",
                "zone_name": zone_name,
                "sn_url": "",
                "sn_host": "",
                "gateway_type": "PortForward",
                "public_key": owner_public_key,
                "private_key": owner_private_key,
                "device_public_key": public_key,
                "device_private_key": private_key,
                "guest_access": True,
                "admin_password_hash": hash_password("test", "testtest"),
            }
            await do_active(ip, req)
            time.sleep(3)

            device = remote_device("nodeA2")
            device.run_command('sudo nohup /opt/buckyos/bin/node_daemon/node_daemon --enable_active > /dev/null 2>&1 &')
            time.sleep(10)


@pytest.mark.asyncio
async def test_active_with_sn(init_context):
    nodes = init_context

    sn_ip = None
    for node_id, node in nodes.items():
        if node_id == 'sn':
            sn_ip = node.get('ipv4')[0]
            break

    if sn_ip is None:
        assert False, "sn node not found"

    reset_sn_server('sn', sn_ip)

    for node_id, node in nodes.items():
        if node_id == 'nodeB1':
            reset_active('nodeB1')
            ip = node.get('ipv4')[0]
            owner_public_key, owner_private_key = await generate_key_pair(ip)

            public_key, private_key = await generate_key_pair(ip)

            zone_boot_config_jwt = await generate_zone_boot_config_jwt(ip, "", owner_private_key)
            print(zone_boot_config_jwt)

            await regsiter_sn_user(sn_ip, None, None, None, None, assert_failed=False, sn_domain="web3.buckyos.io")
            await regsiter_sn_user(sn_ip, "test", "11111", json.dumps(owner_public_key), zone_boot_config_jwt, sn_domain="web3.buckyos.io")

            req = {
                "sn_url": "",
                "sn_host": "",
                "guest_access": True,
                "admin_password_hash": hash_password("test", "testtest"),
            }
            await do_active(ip, req, False)

            req = {
                "user_name": "test",
                "zone_name": "test.web3.buckyos.io",
                "sn_url": "",
                "sn_host": "",
                "gateway_type": "BuckyForward",
                "public_key": "sdfsdfasdf",
                "private_key": "sdfsdfasdf",
                "device_public_key": "sdfsdfasdf",
                "device_private_key": "sdfsdfasdf",
                "guest_access": True,
                "admin_password_hash": hash_password("test", "testtest"),
            }
            await do_active(ip, req, False)

            req = {
                "user_name": "test",
                "zone_name": "test.web3.buckyos.io",
                "sn_url": "http://web3.buckyos.io/kapi/sn",
                "sn_host": "web3.buckyos.io",
                "gateway_type": "BuckyForward",
                "public_key": owner_public_key,
                "private_key": owner_private_key,
                "device_public_key": public_key,
                "device_private_key": private_key,
                "guest_access": True,
                "admin_password_hash": hash_password("test", "testtest"),
            }
            await do_active(ip, req)

            resp = query_with_dns("test.web3.buckyos.io", dns_server=sn_ip)
            print(f'test.web3.buckyos.io ips {resp}')

            resp = query_with_dns("ood1.test.web3.buckyos.io", dns_server=sn_ip)
            print(f'ood1.test.web3.buckyos.io ips {resp}')


@pytest.mark.asyncio
async def test_active_with_sn_invalid(init_context):
    nodes = init_context

    kill_sn_server('sn')

    sn_ip = None
    for node_id, node in nodes.items():
        if node_id == 'sn':
            sn_ip = node.get('ipv4')[0]
            break

    if sn_ip is None:
        assert False, "sn node not found"

    for node_id, node in nodes.items():
        if node_id == 'nodeB1':
            reset_active('nodeB1')
            ip = node.get('ipv4')[0]
            owner_public_key, owner_private_key = await generate_key_pair(ip)

            public_key, private_key = await generate_key_pair(ip)

            zone_boot_config_jwt = await generate_zone_boot_config_jwt(ip, "", owner_private_key)
            print(zone_boot_config_jwt)

            req = {
                "user_name": "test",
                "zone_name": "test.web3.buckyos.io",
                "sn_url": "http://web3.buckyos.io/kapi/sn",
                "sn_host": "web3.buckyos.io",
                "gateway_type": "BuckyForward",
                "public_key": owner_public_key,
                "private_key": owner_private_key,
                "device_public_key": public_key,
                "device_private_key": private_key,
                "guest_access": True,
                "admin_password_hash": hash_password("test", "testtest"),
            }
            await do_active(ip, req, check_success=False)

            resp = query_with_dns("test.web3.buckyos.io", dns_server=sn_ip)
            print(f'test.web3.buckyos.io ips {resp}')

            resp = query_with_dns("ood1.test.web3.buckyos.io", dns_server=sn_ip)
            print(f'ood1.test.web3.buckyos.io ips {resp}')
