from enum import Enum
import json
from typing import Any, Optional, Union
import time
import requests
from urllib3.util import connection
import dns.resolver

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


class RPCProtocolType(Enum):
    HttpPostJson = 'HttpPostJson'


class RPCError(Exception):
    def __init__(self, message: str):
        super().__init__(message)
        self.name = 'RPCError'


class kRPCClient:
    def __init__(self, url: str, token: Optional[str] = None, seq: Optional[int] = None):
        self.server_url = url
        self.protocol_type = RPCProtocolType.HttpPostJson
        # Use milliseconds timestamp as initial sequence number
        self.seq = seq if seq is not None else int(time.time() * 1000)
        self.session_token = token
        self.init_token = token

    async def call(self, method: str, params: Any) -> Any:
        return await self._call(method, params)

    def set_seq(self, seq: int) -> None:
        self.seq = seq

    async def _call(self, method: str, params: Any) -> Any:
        current_seq = self.seq
        self.seq += 1

        request_body = {
            "method": method,
            "params": params,
            "sys": [current_seq, self.session_token] if self.session_token else [current_seq]
        }

        try:
            response = requests.post(
                self.server_url,
                headers={'Content-Type': 'application/json'},
                data=json.dumps(request_body)
            )

            if not response.ok:
                raise RPCError(f"RPC call error: {response.status_code}")

            rpc_response = response.json()

            if "sys" in rpc_response:
                sys = rpc_response["sys"]
                if not isinstance(sys, list):
                    raise RPCError("sys is not array")

                if len(sys) > 1:
                    response_seq = sys[0]
                    if not isinstance(response_seq, int):
                        raise RPCError("sys[0] is not number")
                    if response_seq != current_seq:
                        raise RPCError(f"seq not match: {response_seq}!={current_seq}")

                if len(sys) > 2:
                    token = sys[1]
                    if not isinstance(token, str):
                        raise RPCError("sys[1] is not string")
                    self.session_token = token

            if "error" in rpc_response:
                raise RPCError(f"RPC call error: {rpc_response['error']}")

            return rpc_response.get("result")

        except requests.exceptions.RequestException as e:
            raise RPCError(f"RPC call failed: {str(e)}")
        except json.JSONDecodeError as e:
            raise RPCError(f"RPC response parsing failed: {str(e)}")
        except Exception as e:
            raise RPCError(f"Unexpected error: {str(e)}")


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
