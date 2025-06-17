from urllib3.util import connection
import dns.resolver
from buckyos import kRPCClient

_orig_create_connection = connection.create_connection

domain_ip_map = dict()
def patched_create_connection(address, *args, **kwargs):
    host, port = address
    global domain_ip_map
    if host in domain_ip_map:
        return _orig_create_connection((domain_ip_map[host], port), *args, **kwargs)
    return _orig_create_connection((host, port), *args, **kwargs)

# 应用补丁
connection.create_connection = patched_create_connection


def register_domain_ip(domain: str, ip: str):
    global domain_ip_map
    domain_ip_map[domain] = ip



async def sn_check_active_code(sn_server: str, active_code: str, assert_failed: bool = True, sn_domain: str = "web3.buckyos.ai"):
    result = None
    try:
        register_domain_ip(sn_domain, sn_server)
        client = kRPCClient(f'http://{sn_domain}/kapi/sn')
        result = await client.call("check_active_code", {"active_code": active_code})
    except Exception as e:
        if assert_failed:
            assert False, f"check_active_code failed: {str(e)}"
        else:
            return

    if assert_failed:
        assert result['valid'] == True
    else:
        assert result['valid'] == False

async def sn_check_username(sn_server: str, username: str, assert_failed: bool = True, sn_domain: str = "web3.buckyos.ai"):
    result = None
    try:
        register_domain_ip(sn_domain, sn_server)
        client = kRPCClient(f'http://{sn_domain}/kapi/sn')
        result = await client.call("check_username", {"username": username})
    except Exception as e:
        if assert_failed:
            assert False, f"check user failed: {str(e)}"
        else:
            return

    print(f"check_username {username} result: {result}")
    if assert_failed:
        assert result['valid'] == True
    else:
        assert result['valid'] == False

async def regsiter_sn_user(sn_server: str, user_name: str, active_code: str, public_key: str, zone_config_jwt: str, user_domain: str = None, assert_failed: bool = True, sn_domain: str = "web3.buckyos.ai"):
    try:
        register_domain_ip(sn_domain, sn_server)
        rpc = kRPCClient(f'http://{sn_domain}/kapi/sn')
        req = {
            "user_name": user_name,
            "active_code": active_code,
            "public_key": public_key,
            "zone_config": zone_config_jwt,
        }
        if user_domain is not None:
            req['user_domain'] = user_domain

        await rpc.call('register_user', req)
    except Exception as e:
        if assert_failed:
            assert False, f"register_user failed: {e}"
        return

    if not assert_failed:
        assert False, f"register_user should failed"

async def sn_register_device(sn_server: str, user_name: str, device_name: str, device_did: str, device_ip: str, device_info: str, assert_failed: bool = True, sn_domain: str = "web3.buckyos.ai"):
    try:
        register_domain_ip(sn_domain, sn_server)
        rpc = kRPCClient(f'http://{sn_domain}/kapi/sn')
        req = {
            "user_name": user_name,
            "device_name": device_name,
            "device_did": device_did,
            "device_ip": device_ip,
            "device_info": device_info,
        }
        await rpc.call('register_device', req)
    except Exception as e:
        if assert_failed:
            assert False, f"register_device failed: {e}"
        return

    if not assert_failed:
        assert False, f"register_device should failed"


def query_with_dns(domain, dns_server="8.8.8.8", record_type="A", dns_port=53) -> list[str] | None:
    resolver = dns.resolver.Resolver()
    resolver.retry_servfail = False
    resolver.nameservers = [dns_server]  # 指定DNS服务器
    resolver.port = dns_port  # 指定DNS服务器端口
    try:
        answers = resolver.resolve(domain, record_type, raise_on_no_answer=False)
        records = []
        for record in answers:
            records.append(record.to_text())
        return records
    except Exception as e:
        print(f"DNS query failed: {e}")
        return None
