from enum import Enum
import json
from typing import Any, Optional, Union
import time
import requests


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

