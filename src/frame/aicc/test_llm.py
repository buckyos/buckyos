#!/usr/bin/env python3
import json
import os
import time
import unittest
import uuid
import urllib.error
import urllib.request


AICC_URL = os.environ.get("AICC_URL", "http://127.0.0.1:4040/kapi/aicc")
AICC_MODEL_ALIAS = os.environ.get("AICC_MODEL_ALIAS", "llm.default")
AICC_RPC_TOKEN = os.environ.get("AICC_RPC_TOKEN")
AICC_TIMEOUT_SECONDS = float(os.environ.get("AICC_TIMEOUT_SECONDS", "90"))
AICC_TEST_INPUT = os.environ.get(
    "AICC_TEST_INPUT",
    "医生的免疫记录本里，写H，是指得过的意思么？",
)


class AiccRpcError(RuntimeError):
    pass


def _build_sys(seq: int, token: str | None = None, trace_id: str | None = None) -> list:
    sys_arr: list = [seq]
    if token is not None or trace_id is not None:
        sys_arr.append(token)
    if trace_id is not None:
        sys_arr.append(trace_id)
    return sys_arr


def call_rpc(method: str, params: dict, seq: int = 1) -> dict:
    body = {
        "method": method,
        "params": params,
        "sys": _build_sys(seq, token=AICC_RPC_TOKEN),
    }
    data = json.dumps(body).encode("utf-8")
    request = urllib.request.Request(
        AICC_URL,
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )

    try:
        with urllib.request.urlopen(request, timeout=AICC_TIMEOUT_SECONDS) as response:
            payload = response.read().decode("utf-8")
    except urllib.error.URLError as err:
        raise AiccRpcError(f"request to {AICC_URL} failed: {err}") from err

    try:
        rpc_response = json.loads(payload)
    except json.JSONDecodeError as err:
        raise AiccRpcError(f"invalid JSON response: {payload}") from err

    if "error" in rpc_response:
        raise AiccRpcError(f"rpc error: {rpc_response['error']}")

    if "result" not in rpc_response:
        raise AiccRpcError(f"rpc response missing result field: {rpc_response}")

    return rpc_response["result"]


def complete_once(prompt: str, must_features: list[str] | None = None, options: dict | None = None) -> dict:
    params = {
        "capability": "llm_router",
        "model": {
            "alias": AICC_MODEL_ALIAS,
        },
        "requirements": {
            "must_features": must_features or [],
        },
        "payload": {
            "messages": [
                {
                    "role": "user",
                    "content": prompt,
                }
            ],
            "options": options or {"temperature": 0.2, "max_tokens": 96},
        },
        "idempotency_key": f"test-{uuid.uuid4().hex}",
    }
    return call_rpc("complete", params, seq=int(time.time()))


class TestAiccOpenAIProvider(unittest.TestCase):
    def test_complete_basic(self) -> None:
        result = complete_once("Reply with one short sentence: AICC OpenAI provider is working.")

        self.assertIn("task_id", result, msg=f"missing task_id in result: {result}")
        self.assertIn("status", result, msg=f"missing status in result: {result}")
        self.assertIn(result["status"], ["succeeded", "running"], msg=f"unexpected status: {result}")

        if result["status"] == "succeeded":
            self.assertIsInstance(result.get("result"), dict, msg=f"invalid result payload: {result}")
            summary = result.get("result") or {}
            has_text = isinstance(summary.get("text"), str) and len(summary.get("text", "").strip()) > 0
            has_json = summary.get("json") is not None
            self.assertTrue(
                has_text or has_json,
                msg=f"succeeded response should include text/json summary: {summary}",
            )

    def test_complete_json_output(self) -> None:
        result = complete_once(
            prompt='Return JSON only with shape {"ok": true, "source": "aicc"}.',
            must_features=["json_output"],
            options={
                "temperature": 0,
                "max_tokens": 80,
                "response_format": {"type": "json_object"},
            },
        )

        self.assertIn(result["status"], ["succeeded", "running"], msg=f"unexpected status: {result}")
        if result["status"] == "succeeded":
            summary = result.get("result") or {}
            parsed = summary.get("json")
            if parsed is None and isinstance(summary.get("text"), str):
                parsed = json.loads(summary["text"])

            self.assertIsInstance(parsed, dict, msg=f"json_output should return JSON object: {summary}")
            self.assertIn("ok", parsed, msg=f"json result missing 'ok': {parsed}")

    def test_cancel_endpoint_reachable(self) -> None:
        cancel_result = call_rpc(
            "cancel",
            {"task_id": f"test-task-{uuid.uuid4().hex}"},
            seq=int(time.time()),
        )

        self.assertIn("task_id", cancel_result, msg=f"missing task_id in cancel response: {cancel_result}")
        self.assertIn("accepted", cancel_result, msg=f"missing accepted in cancel response: {cancel_result}")
        self.assertIsInstance(cancel_result["accepted"], bool)

    def test_input_and_print_output(self) -> None:
        result = complete_once(
            prompt=AICC_TEST_INPUT,
            options={"temperature": 0.2, "max_tokens": 256},
        )

        status = result.get("status")
        self.assertIn(status, ["succeeded", "running"], msg=f"unexpected status: {result}")

        summary = result.get("result") or {}
        output_text = summary.get("text")
        output_json = summary.get("json")

        print("\n=== AICC Manual Input Test ===")
        print(f"Input: {AICC_TEST_INPUT}")
        print(f"Status: {status}")
        if isinstance(output_text, str) and output_text.strip():
            print("Output(text):")
            print(output_text.strip())
        elif output_json is not None:
            print("Output(json):")
            print(json.dumps(output_json, ensure_ascii=False, indent=2))
        else:
            print("Output: <none yet, task may still be running>")


if __name__ == "__main__":
    unittest.main(verbosity=2)
