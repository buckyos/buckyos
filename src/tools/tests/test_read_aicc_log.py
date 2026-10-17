import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SCRIPT_PATH = Path(__file__).parents[2] / "read_aicc_log.py"


def load_script():
    spec = importlib.util.spec_from_file_location("buckyos_read_aicc_log", SCRIPT_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def log_line(direction, event):
    return (
        "10-17 07:08:09.123 INFO [execution/mod.rs:123] "
        f"aicc.llm.{direction} {json.dumps(event, ensure_ascii=False)}\n"
    )


class ReadAiccLogTests(unittest.TestCase):
    def setUp(self):
        self.reader = load_script()
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.logfile = Path(self.temporary.name) / "aicc.20261017.log"

    def parse(self, lines, show_errors=False):
        self.logfile.write_text("".join(lines), encoding="utf-8")
        return self.reader.parse_entries(str(self.logfile), show_errors)

    def test_canonical_request_preserves_tools_results_and_thinking(self):
        request = {
            "exact_model": "provider/model",
            "messages": [
                {"role": "system", "content": [{"type": "text", "text": "系统要求"}]},
                {
                    "role": "assistant",
                    "content": [
                        {"type": "thinking", "summary": "检查输出", "text": "先运行工具"},
                        {"type": "text", "text": "正在检查"},
                        {
                            "type": "tool_use", "call_id": "call_1", "name": "shell",
                            "args": {"command": "echo hello\nprintf done", "timeout": 3},
                        },
                    ],
                },
                {
                    "role": "tool",
                    "content": [{
                        "type": "tool_result", "call_id": "call_1", "is_error": True,
                        "content": [
                            {"type": "text", "text": "命令失败"},
                            {"type": "document", "title": "result.txt", "source": {"url": "https://example.test/result"}},
                        ],
                    }],
                },
            ],
            "tools": [{"type": "function", "name": "shell", "description": "执行命令", "args_json_schema": {"type": "object"}}],
        }
        cleaned = self.reader.extract_clean_input(request)
        self.assertEqual(cleaned["model"], "provider/model")
        self.assertEqual(cleaned["tools"], request["tools"])
        assistant = cleaned["messages"][1]["content"]
        for text in ("[THINKING]", "检查输出", "先运行工具", "正在检查", "call_id=call_1", "echo hello\nprintf done"):
            self.assertIn(text, assistant)
        self.assertLess(assistant.index("先运行工具"), assistant.index("正在检查"))
        tool_result = cleaned["messages"][2]["content"]
        for text in ("TOOL_RESULT", "call_id=call_1", "is_error=true", "命令失败", "result.txt"):
            self.assertIn(text, tool_result)

    def test_canonical_response_deduplicates_embedded_tool_calls(self):
        call = {"call_id": "call_1", "name": "lookup", "args": {"query": "天气"}}
        response = {
            "message": {"role": "assistant", "content": [{"type": "tool_use", **call}]},
            "tool_calls": [call, {"call_id": "call_2", "name": "finish", "args": {}}],
            "finish_reason": "tool_calls", "status": "completed",
        }
        replies = self.reader.extract_clean_output(response)["replies"]
        rendered = "\n".join(reply["content"] for reply in replies)
        self.assertEqual(rendered.count("call_id=call_1"), 1)
        self.assertEqual(rendered.count("call_id=call_2"), 1)
        self.assertIn('lookup({"query": "天气"})', rendered)
        self.assertTrue(all(reply["finish_reason"] == "tool_calls" for reply in replies))

    def test_null_and_empty_content_render_without_none(self):
        for content in (None, "", []):
            with self.subTest(content=content):
                message = {"role": "assistant", "content": content}
                self.assertEqual(self.reader._extract_message_content(message), "")
                response = {"message": message, "status": "completed"}
                rendered = self.reader.render_llm_entry_html(1, "time", "output", {}, response, False)
                self.assertNotIn("None", rendered)
                with contextlib.redirect_stdout(io.StringIO()) as stdout:
                    self.reader.display_entry("time", "output", {}, response)
                self.assertNotIn("None", stdout.getvalue())
        self.assertEqual(self.reader.extract_clean_output({"message": None, "tool_calls": [], "usage": None}), {})
        self.assertEqual(self.reader.format_message_content(None), "")
        self.assertEqual(json.loads(self.reader.format_message_content({"ok": False})), {"ok": False})

    def test_usage_preserves_zero_and_does_not_double_count_cache(self):
        usage = {
            "input_tokens": 0, "output_tokens": 12, "cache_read_input_tokens": 7,
            "cache_write_input_tokens": 3, "cache_write_1h_input_tokens": 2,
            "reasoning_tokens": 5,
        }
        output = {"usage": usage}
        clean_usage = self.reader.extract_clean_output(output)["usage"]
        self.assertEqual(clean_usage["prompt_tokens"], 0)
        self.assertEqual(clean_usage["total_tokens"], 12)
        entries = [("llm", ("time", "output", {"exact_model": "provider/model"}, output))]
        stats = self.reader.collect_html_stats(entries)
        self.assertEqual(stats["total_tokens"], 12)
        self.assertEqual(stats["cached_tokens"], 7)
        self.assertEqual(stats["cache_write_input_tokens"], 3)
        self.assertEqual(stats["cache_write_1h_input_tokens"], 2)
        self.assertEqual(stats["reasoning_tokens"], 5)
        self.assertEqual(stats["models"], {"provider/model": 1})
        zeros = self.reader.extract_clean_output({"usage": {"input_tokens": 0, "prompt_tokens": 9, "output_tokens": 0, "completion_tokens": 4, "total_tokens": 0}})["usage"]
        self.assertEqual((zeros["prompt_tokens"], zeros["completion_tokens"], zeros["total_tokens"]), (0, 0, 0))

    def test_provider_payloads_render_reasoning_tools_and_usage(self):
        request = {"input": [
            {"type": "reasoning", "summary": [{"type": "summary_text", "text": "先搜索"}]},
            {"type": "function_call", "call_id": "call_1", "name": "lookup", "arguments": '{"query":"天气"}'},
            {"type": "function_call_output", "call_id": "call_1", "output": {"ok": False}},
        ]}
        messages = self.reader.extract_clean_input(request)["messages"]
        self.assertIn("先搜索", messages[0]["content"])
        self.assertIn("call_id=call_1", messages[1]["title"])
        self.assertEqual(json.loads(messages[2]["content"]), {"ok": False})
        response = {
            "choices": [{"message": {"role": "assistant", "content": None, "reasoning_content": "分析完成"}}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 2, "prompt_tokens_details": {"cached_tokens": 4}, "completion_tokens_details": {"reasoning_tokens": 1}},
        }
        clean = self.reader.extract_clean_output(response)
        self.assertIn("分析完成", clean["replies"][0]["content"])
        self.assertEqual(clean["usage"]["cached_tokens"], 4)
        self.assertEqual(clean["usage"]["reasoning_tokens"], 1)

    def test_task_id_correlates_interleaved_replies_and_actual_provider_wins(self):
        shared = {"provider": "openai", "provider_instance_name": "primary", "trace_id": "shared-trace", "exact_model": "openai/model"}
        entries = self.parse([
            log_line("input", {**shared, "task_id": "task_1", "tenant_id": "first user", "request": {"messages": []}}),
            log_line("input", {**shared, "task_id": "task_2", "tenant_id": "second user", "request": {"messages": []}}),
            log_line("output", {"task_id": "task_2", "provider": "claude", "provider_instance_name": "fallback", "exact_model": "claude/model", "response": {"status": "completed"}}),
            log_line("output", {"task_id": "task_1", "response": {"status": "completed"}}),
        ])
        second = entries[2][1][2]
        first = entries[3][1][2]
        self.assertEqual(second["tenant_id"], "second user")
        self.assertEqual(first["tenant_id"], "first user")
        self.assertEqual(second["trace_id"], "shared-trace")
        self.assertEqual((second["provider"], second["provider_instance_name"], second["exact_model"]), ("claude", "fallback", "claude/model"))
        self.assertEqual(first["provider_instance_name"], "primary")

    def test_errors_are_optional_and_inherit_task_context(self):
        lines = [
            log_line("input", {"task_id": "task_1", "caller_app_id": "jarvis", "request": {"messages": []}}),
            log_line("error", {"task_id": "task_1", "error": {"code": "provider_error", "message": "failed <request>"}}),
        ]
        self.assertEqual(len(self.parse(lines)), 1)
        entries = self.parse(lines, show_errors=True)
        event = entries[1][1][1]
        self.assertEqual(entries[1][0], "error")
        self.assertEqual(event["caller_app_id"], "jarvis")
        rendered = self.reader.render_error_html(2, "time", event)
        self.assertIn("failed &lt;request&gt;", rendered)
        self.assertIn("jarvis", rendered)
        with contextlib.redirect_stdout(io.StringIO()) as stdout:
            self.reader.display_error("time", event)
        self.assertIn("failed <request>", stdout.getvalue())
        self.assertEqual(self.reader.collect_html_stats(entries)["error_entries"], 1)

    def test_explicit_null_error_route_does_not_inherit_primary_provider(self):
        entries = self.parse([
            log_line("input", {
                "task_id": "task_1", "caller_app_id": "jarvis",
                "provider": "openai", "provider_instance_name": "primary",
                "exact_model": "openai/model", "request": {"messages": []},
            }),
            log_line("error", {
                "task_id": "task_1", "provider": None,
                "provider_instance_name": None, "exact_model": None,
                "error": {"code": "provider_error", "message": "fallback failed before accept"},
            }),
        ], show_errors=True)
        event = entries[1][1][1]
        self.assertEqual(event["caller_app_id"], "jarvis")
        for key in ("provider", "provider_instance_name", "exact_model"):
            self.assertIsNone(event[key])
        rendered = self.reader.render_error_html(2, "time", event)
        with contextlib.redirect_stdout(io.StringIO()) as stdout:
            self.reader.display_error("time", event)
        for text in (rendered, stdout.getvalue()):
            self.assertNotIn("primary", text)
            self.assertNotIn("openai", text)
            self.assertIn("fallback failed before accept", text)

    def test_truncated_and_nonobject_payloads_remain_visible(self):
        lines = [
            '10-17 07:08:09.123 INFO [execution/mod.rs:123] aicc.llm.input {"task_id":"truncated","request":\n',
            log_line("output", {"task_id": "invalid", "response": None}),
            log_line("output", ["unexpected", "payload"]),
        ]
        entries = self.parse(lines)
        self.assertEqual(len(entries), 3)
        for _, data in entries:
            with self.subTest(data=data):
                timestamp, direction, meta, payload = data
                self.assertIn("_raw", payload)
                rendered = self.reader.render_llm_entry_html(1, timestamp, direction, meta, payload, False)
                self.assertIn("_raw", rendered)
                with contextlib.redirect_stdout(io.StringIO()) as stdout:
                    self.reader.display_entry(timestamp, direction, meta, payload)
                self.assertIn("_raw", stdout.getvalue())
        self.assertIn("truncated", self.reader.render_html_report(str(self.logfile), entries, False, "summary"))

    def test_cli_renders_html_and_plain_raw_terminal(self):
        self.parse([
            log_line("input", {"task_id": "task_1", "exact_model": "provider/model", "request": {"messages": [{"role": "user", "content": [{"type": "text", "text": "你好 <script>"}]}]}}),
            log_line("output", {"task_id": "task_1", "response": {"message": {"role": "assistant", "content": [{"type": "text", "text": "已完成"}]}, "usage": {"input_tokens": 0, "output_tokens": 3}, "status": "completed"}}),
            log_line("error", {"task_id": "task_2", "error": {"code": "provider_error", "message": "测试错误"}}),
        ])
        with patch.object(self.reader.sys, "argv", [str(SCRIPT_PATH), str(self.logfile), "--errors"]), contextlib.redirect_stdout(io.StringIO()) as stdout:
            self.reader.main()
        report = self.logfile.with_name("aicc_log.html").read_text(encoding="utf-8")
        for text in ("你好 &lt;script&gt;", "已完成", "测试错误", "tokens total=3", "provider/model"):
            self.assertIn(text, report)
        self.assertNotIn("你好 <script>", report)
        self.assertIn("HTML report:", stdout.getvalue())
        raw_report = self.reader.render_html_report(
            str(self.logfile), self.reader.parse_entries(str(self.logfile), True), True, "summary"
        )
        self.assertIn('"messages": [', raw_report)
        self.assertIn("你好 &lt;script&gt;", raw_report)
        self.assertIn("测试错误", raw_report)
        for extra_flags in ([], ["--raw"]):
            with self.subTest(flags=extra_flags):
                argv = [str(SCRIPT_PATH), str(self.logfile), "--print", "--no-color", "--errors", *extra_flags]
                with patch.object(self.reader.sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()) as stdout:
                    self.reader.main()
                text = stdout.getvalue()
                self.assertNotIn("\033[", text)
                for expected in ("你好 <script>", "已完成", "测试错误", "2 LLM log entries", "1 errors"):
                    self.assertIn(expected, text)
                if extra_flags:
                    self.assertIn('"messages": [', text)


if __name__ == "__main__":
    unittest.main()
