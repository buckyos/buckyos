#!/usr/bin/env python3
"""
Parse LLM log files and display formatted prompts/responses.

Extracts aicc.llm.input and aicc.llm.output JSON events,
and presents the content in readable HTML by default.

Usage:
    python read_aicc_log.py [logfile] [--raw] [--errors]
    python read_aicc_log.py [logfile] --print [--raw] [--no-color] [--errors]

    If no logfile is given, automatically finds the latest aicc log under
    $BUCKYOS_ROOT/logs/aicc/ (defaults to /opt/buckyos/logs/aicc/).

Options:
    --print     Print to terminal instead of generating HTML
    --raw       Show full JSON instead of cleaned-up view
    --no-color  Disable ANSI color codes in --print mode
    --errors    Also show aicc.llm.error events
"""

import sys
import json
import re
import os
import glob
import html
from pathlib import Path
from typing import Optional

# ANSI colors (will be cleared if --no-color)
C_RESET = "\033[0m"
C_BOLD = "\033[1m"
C_DIM = "\033[2m"
C_CYAN = "\033[36m"
C_GREEN = "\033[32m"
C_YELLOW = "\033[33m"
C_RED = "\033[31m"
C_BLUE = "\033[34m"
C_MAGENTA = "\033[35m"
C_WHITE = "\033[97m"
C_GRAY = "\033[90m"

ROLE_COLORS = {
    "system": C_MAGENTA,
    "user": C_GREEN,
    "assistant": C_CYAN,
    "tool": C_YELLOW,
    "tool_call": C_YELLOW,
    "tool_result": C_YELLOW,
    "error": C_RED,
}

DEFAULT_BUCKYOS_ROOT = "/opt/buckyos"
LOG_PREFIX_RE = (
    r"^(\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+)\s+"
    r"(\w+)\s+"
    r"\[\S+\]\s+"
)


def find_latest_aicc_log() -> Optional[str]:
    """Find the most recently modified aicc log file under $BUCKYOS_ROOT/logs/aicc/."""
    root = os.environ.get("BUCKYOS_ROOT", "").strip() or DEFAULT_BUCKYOS_ROOT
    log_dir = os.path.join(root, "logs", "aicc")
    pattern = os.path.join(log_dir, "aicc.*.log")
    files = glob.glob(pattern)
    if not files:
        return None
    return max(files, key=os.path.getmtime)


def parse_log_line(line: str):
    parsed = parse_event_line(line)
    if not parsed:
        return None
    timestamp, direction, event = parsed
    if direction not in ("input", "output"):
        return None
    key = "request" if direction == "input" else "response"
    payload = event.get(key, {"_raw": event})
    if not isinstance(payload, dict):
        payload = {"_raw": payload}
    meta = {key_: value for key_, value in event.items() if key_ != key}
    return timestamp, direction, meta, payload


def parse_error_line(line: str):
    parsed = parse_event_line(line)
    if parsed and parsed[1] == "error":
        return parsed[0], parsed[2]
    return None


def parse_event_line(line: str):
    m = re.match(
        LOG_PREFIX_RE + r"aicc\.llm\.(input|output|error)\s+(.+)$",
        line.strip(),
    )
    if not m:
        return None
    event = _try_repair_json(m.group(4))
    if not isinstance(event, dict):
        event = {"_raw": event}
    return m.group(1), m.group(3), event


def _try_repair_json(raw: str) -> dict:
    """Attempt to repair and parse malformed JSON from logs."""
    decoder = json.JSONDecoder()
    try:
        obj, _ = decoder.raw_decode(raw)
        return obj
    except json.JSONDecodeError:
        pass
    return {"_raw": raw}


def _extract_message_content(msg: dict) -> str:
    """Extract text content from a message in either API format."""
    content = msg.get("content", "")
    if content is None:
        return ""
    if isinstance(content, str):
        return content
    if isinstance(content, dict):
        content = [content]
    if isinstance(content, list):
        parts = []
        for block in content:
            if isinstance(block, dict):
                block_type = block.get("type")
                if block_type in ("tool_use", "function_call"):
                    call_id = block.get("call_id") or block.get("id")
                    label = "TOOL_CALL" + (f" call_id={call_id}" if call_id else "")
                    parts.append(f"[{label}]\n{_format_function_call_content(block)}")
                elif block_type in ("tool_result", "function_call_output"):
                    call_id = block.get("call_id") or block.get("tool_use_id")
                    label = "TOOL_RESULT" + (f" call_id={call_id}" if call_id else "")
                    if block.get("is_error"):
                        label += " is_error=true"
                    result = block.get("content", block.get("output"))
                    parts.append(f"[{label}]\n{_extract_message_content({'content': result})}")
                elif block_type in ("thinking", "reasoning"):
                    texts = []
                    for key in ("summary", "text", "thinking"):
                        value = _extract_message_content({"content": block.get(key)})
                        if value and value not in texts:
                            texts.append(value)
                    parts.append("[THINKING]" + ("\n" + "\n".join(texts) if texts else ""))
                elif block_type == "provider_state":
                    provider = block.get("provider", "")
                    parts.append(f"[provider_state {provider}]\n{format_message_content(block.get('value'))}")
                elif block.get("text") is not None:
                    parts.append(format_message_content(block["text"]))
                elif block.get("output") is not None:
                    parts.append(format_message_content(block["output"]))
                elif block_type:
                    title = block.get("title")
                    parts.append(f"[{block_type}" + (f": {title}" if title else "") + "]")
                else:
                    parts.append(format_message_content(block))
            elif isinstance(block, str):
                parts.append(block)
        return "\n".join(parts)
    return format_message_content(content)


def _format_function_call_content(item: dict) -> str:
    item = item.get("function") or item
    name = item.get("name", "?")
    arguments = item.get("args", item.get("arguments", ""))
    try:
        parsed = json.loads(arguments) if isinstance(arguments, str) else arguments
        if name == "shell" and isinstance(parsed, dict):
            return _format_shell_call(parsed)
        arguments = json.dumps(parsed, ensure_ascii=False)
    except (json.JSONDecodeError, TypeError):
        pass
    return f"-> {name}({arguments})"


def _format_shell_call(args: dict) -> str:
    command = args.get("command")
    rest = {key: value for key, value in args.items() if key != "command"}
    lines = ["-> shell"]
    if rest:
        lines.append(f"args: {json.dumps(rest, ensure_ascii=False, sort_keys=True)}")
    if isinstance(command, str):
        lines.append("command:")
        lines.append(command)
    elif command is not None:
        lines.append(f"command: {json.dumps(command, ensure_ascii=False)}")
    return "\n".join(lines)


def extract_clean_input(payload: dict) -> dict:
    """Extract readable content from an llm.input request payload."""
    result = {}

    model = payload.get("exact_model") or payload.get("logical_model") or payload.get("model")
    if model:
        result["model"] = model

    raw_messages = payload.get("messages") or payload.get("input") or []
    if isinstance(raw_messages, str):
        raw_messages = [{"role": "user", "content": raw_messages}]
    elif isinstance(raw_messages, dict):
        raw_messages = [raw_messages]

    clean_messages = []
    for msg in raw_messages:
        if not isinstance(msg, dict):
            clean_messages.append({"role": "user", "content": format_message_content(msg)})
            continue
        item_type = msg.get("type")
        if item_type == "function_call":
            call_id = msg.get("call_id", "")
            title = "TOOL_CALL"
            if call_id:
                title += f" call_id={call_id}"
            clean_messages.append({
                "role": "tool_call",
                "title": title,
                "content": _format_function_call_content(msg),
            })
            continue
        if item_type == "function_call_output":
            call_id = msg.get("call_id", "")
            title = "TOOL_RESULT"
            if call_id:
                title += f" call_id={call_id}"
            clean_messages.append({
                "role": "tool_result",
                "title": title,
                "content": _extract_message_content({"content": msg.get("output")}),
            })
            continue
        if item_type == "reasoning":
            clean_messages.append({"role": "assistant", "content": _extract_message_content({"content": [msg]})})
            continue
        role = msg.get("role", "unknown")
        content = _extract_message_content(msg)
        for call in msg.get("tool_calls") or []:
            block = dict(call, type="function_call")
            content += ("\n" if content else "") + _extract_message_content({"content": [block]})
        if msg.get("tool_call_id"):
            content = f"[TOOL_RESULT call_id={msg['tool_call_id']}]\n{content}"
        clean_messages.append({"role": role, "content": content})

    if clean_messages:
        result["messages"] = clean_messages

    tools = payload.get("tools") or payload.get("functions")
    if tools:
        result["tools"] = tools

    return result


def _extract_usage(usage: dict) -> dict:
    result = dict(usage)
    input_details = usage.get("input_tokens_details") or usage.get("prompt_tokens_details") or {}
    output_details = usage.get("output_tokens_details") or usage.get("completion_tokens_details") or {}
    for key, values in {
        "prompt_tokens": (usage.get("input_tokens"), usage.get("prompt_tokens")),
        "completion_tokens": (usage.get("output_tokens"), usage.get("completion_tokens")),
        "cached_tokens": (usage.get("cache_read_input_tokens"), input_details.get("cached_tokens")),
        "reasoning_tokens": (usage.get("reasoning_tokens"), output_details.get("reasoning_tokens")),
    }.items():
        result[key] = next((value for value in values if value is not None), None)
    if result.get("total_tokens") is None:
        counts = (result["prompt_tokens"], result["completion_tokens"])
        result["total_tokens"] = sum(_int_value(value) for value in counts) if any(value is not None for value in counts) else None
    return result


def extract_clean_output(payload: dict) -> dict:
    """Extract readable content from an llm.output response payload."""
    result = {}

    model = payload.get("exact_model") or payload.get("model")
    if model:
        result["model"] = model

    usage = payload.get("usage")
    if usage:
        result["usage"] = _extract_usage(usage)

    replies = []
    message = payload.get("message")
    if isinstance(message, dict):
        replies.append({
            "role": message.get("role", "assistant"),
            "content": _extract_message_content(message),
            "finish_reason": payload.get("finish_reason") or payload.get("status", ""),
        })
    content = message.get("content") if isinstance(message, dict) else []
    embedded_calls = {
        block.get("call_id")
        for block in content or []
        if isinstance(block, dict) and block.get("type") == "tool_use"
    }
    for call in payload.get("tool_calls") or []:
        if call.get("call_id") and call["call_id"] in embedded_calls:
            continue
        replies.append({
            "role": "tool_call",
            "content": _extract_message_content({"content": [dict(call, type="tool_use")]}),
            "finish_reason": payload.get("finish_reason") or payload.get("status", ""),
        })
    if payload.get("error"):
        replies.append({"role": "error", "content": format_message_content(payload["error"])})

    # Chat Completions API format
    for choice in payload.get("choices") or []:
        msg = choice.get("message") or {}
        content = _extract_message_content(msg)
        if msg.get("reasoning_content"):
            content = f"[THINKING]\n{msg['reasoning_content']}\n{content}"
        role = msg.get("role", "assistant")
        entry = {"role": role, "content": content}
        if msg.get("tool_calls"):
            entry["tool_calls"] = msg["tool_calls"]
        if choice.get("finish_reason"):
            entry["finish_reason"] = choice["finish_reason"]
        replies.append(entry)

    # Responses API format
    for item in payload.get("output") or []:
        if item.get("type") == "message":
            role = item.get("role", "assistant")
            status = item.get("status", "")
            content = _extract_message_content(item)
            entry = {"role": role, "content": content}
            if status:
                entry["finish_reason"] = status
            replies.append(entry)

        elif item.get("type") == "function_call":
            entry = {
                "role": "tool_call",
                "content": _format_function_call_content(item),
                "finish_reason": item.get("status", ""),
            }
            replies.append(entry)
        elif item.get("type") == "reasoning":
            replies.append({"role": "assistant", "content": _extract_message_content({"content": [item]})})

    if replies:
        result["replies"] = replies

    return result


def print_separator(char="-", width=88):
    print(f"{C_DIM}{char * width}{C_RESET}")


def _try_format_xml(text: str) -> Optional[str]:
    """Try to pretty-print XML. Returns formatted string or None."""
    stripped = text.strip()
    if not stripped.startswith("<"):
        return None
    try:
        import xml.dom.minidom as minidom
        dom = minidom.parseString(stripped)
        pretty = dom.toprettyxml(indent="  ")
        lines = pretty.splitlines()
        if lines and lines[0].startswith("<?xml"):
            lines = lines[1:]
        return "\n".join(l for l in lines if l.strip())
    except Exception:
        return None


def print_message_content(content: str, role: str, indent: int = 4):
    """Pretty-print a message's content, attempting to parse inner JSON or XML."""
    color = ROLE_COLORS.get(role, C_WHITE)
    prefix = " " * indent
    formatted = format_message_content(content)

    for line in formatted.splitlines():
        print(f"{prefix}{color}{line}{C_RESET}")


def format_message_content(content: str) -> str:
    """Format message content for terminal or HTML display."""
    if content is None:
        return ""
    if not isinstance(content, str):
        return json.dumps(content, indent=2, ensure_ascii=False)
    try:
        inner = json.loads(content)
        return json.dumps(inner, indent=2, ensure_ascii=False)
    except (json.JSONDecodeError, TypeError):
        pass

    xml_formatted = _try_format_xml(content)
    if xml_formatted:
        return xml_formatted

    return str(content)


def display_entry(timestamp, direction, meta, payload, raw=False):
    """Display one log entry in a readable format."""
    print()
    print_separator("=")

    icon = ">> INPUT" if direction == "input" else "<< OUTPUT"
    model = meta.get("exact_model") or payload.get("exact_model") or payload.get("model", "?")
    instance = meta.get("provider_instance_name") or ""
    trace = meta.get("trace_id", "")

    print(f"{C_BOLD}{icon}{C_RESET}  {C_DIM}{timestamp}{C_RESET}  "
          f"{C_YELLOW}{model}{C_RESET}  {C_DIM}{instance}{C_RESET}")

    details = []
    for key in ("task_id", "tenant_id", "caller_app_id", "provider", "method", "request_model", "status"):
        value = meta.get(key) or payload.get(key)
        if value:
            details.append(f"{key}: {value}")
    if details:
        print(f"  {C_DIM}{'  '.join(details)}{C_RESET}")

    if trace and trace != "None":
        print(f"  {C_DIM}trace: {trace}{C_RESET}")

    print_separator()

    if raw or "_raw" in payload:
        print(json.dumps(payload, indent=2, ensure_ascii=False))
        return

    if direction == "input":
        clean = extract_clean_input(payload)
        messages = clean.get("messages", [])
        for i, msg in enumerate(messages):
            role = msg["role"]
            title = msg.get("title", role.upper())
            color = ROLE_COLORS.get(role, C_WHITE)
            print(f"  {C_BOLD}{color}[{title}]{C_RESET}")
            print_message_content(msg["content"], role)
            if i < len(messages) - 1:
                print_separator(".")

        if clean.get("tools"):
            print_separator(".")
            print(f"  {C_BOLD}{C_YELLOW}[TOOLS]{C_RESET}")
            print(f"    {json.dumps(clean['tools'], indent=2, ensure_ascii=False)}")

    elif direction == "output":
        clean = extract_clean_output(payload)

        if clean.get("usage"):
            u = clean["usage"]
            print(f"  {C_DIM}tokens: prompt={u.get('prompt_tokens')} "
                  f"completion={u.get('completion_tokens')} "
                  f"total={u.get('total_tokens')}{C_RESET}")
            print_separator(".")

        for reply in clean.get("replies", []):
            role = reply.get("role", "assistant")
            color = ROLE_COLORS.get(role, C_WHITE)
            print(f"  {C_BOLD}{color}[{role.upper()}]{C_RESET}  "
                  f"{C_DIM}finish={reply.get('finish_reason', '?')}{C_RESET}")
            print_message_content(reply.get("content", ""), role)

            if reply.get("tool_calls"):
                print(f"    {C_YELLOW}tool_calls: "
                      f"{json.dumps(reply['tool_calls'], indent=2, ensure_ascii=False)}{C_RESET}")

    print_separator("=")


def display_error(timestamp, event):
    """Display a provider error entry."""
    print()
    print_separator("=")
    print(f"{C_BOLD}{C_RED}!! ERROR{C_RESET}  {C_DIM}{timestamp}{C_RESET}")
    print_separator()

    for key, value in event.items():
        if key != "error" and value is not None:
            print(f"  {C_DIM}{key}: {value}{C_RESET}")
    print_message_content(event.get("error", event), "error")

    print_separator("=")


def parse_entries(logfile: str, show_errors: bool):
    entries = []
    task_contexts = {}

    with open(logfile, "r", encoding="utf-8") as f:
        for line in f:
            parsed = parse_event_line(line)
            if not parsed:
                continue
            timestamp, direction, event = parsed
            key = {"input": "request", "output": "response", "error": "error"}[direction]
            task_id = event.get("task_id")
            meta = dict(task_contexts.get(task_id, {})) if task_id else {}
            meta.update({k: v for k, v in event.items() if k != key})
            if task_id:
                if direction == "input":
                    task_contexts[task_id] = meta
                else:
                    task_contexts.pop(task_id, None)
            payload = event.get(key, {"_raw": event})
            if direction == "error":
                if show_errors:
                    entries.append(("error", (timestamp, {**meta, "error": payload})))
            else:
                if not isinstance(payload, dict):
                    payload = {"_raw": payload}
                entries.append(("llm", (timestamp, direction, meta, payload)))

    return entries


def summarize_entries(entries):
    llm_count = sum(1 for t, _ in entries if t == "llm")
    err_count = sum(1 for t, _ in entries if t == "error")
    summary = f"Found {llm_count} LLM log entries"
    if err_count:
        summary += f", {err_count} errors"
    return summary, llm_count, err_count


def _int_value(value) -> int:
    return value if isinstance(value, int) else 0


def collect_html_stats(entries: list) -> dict:
    stats = {
        "llm_entries": 0,
        "input_entries": 0,
        "output_entries": 0,
        "error_entries": 0,
        "usage_entries": 0,
        "prompt_tokens": 0,
        "completion_tokens": 0,
        "total_tokens": 0,
        "cached_tokens": 0,
        "cache_write_input_tokens": 0,
        "cache_write_1h_input_tokens": 0,
        "reasoning_tokens": 0,
        "models": {},
    }

    for entry_type, data in entries:
        if entry_type == "error":
            stats["error_entries"] += 1
            continue
        if entry_type != "llm":
            continue

        stats["llm_entries"] += 1
        _, direction, meta, payload = data
        if direction == "input":
            stats["input_entries"] += 1
            continue

        stats["output_entries"] += 1
        model = meta.get("exact_model") or meta.get("model") or payload.get("exact_model") or payload.get("model")
        if model:
            stats["models"][model] = stats["models"].get(model, 0) + 1

        usage = payload.get("usage") or {}
        if not usage:
            continue

        usage = _extract_usage(usage)
        stats["usage_entries"] += 1
        for key in ("prompt_tokens", "completion_tokens", "total_tokens", "cached_tokens",
                    "cache_write_input_tokens", "cache_write_1h_input_tokens", "reasoning_tokens"):
            stats[key] += _int_value(usage.get(key))

    return stats


def format_count(value: int) -> str:
    return f"{value:,}"


def render_stats_text(stats: dict) -> str:
    parts = [
        f"tokens total={format_count(stats['total_tokens'])}",
        f"input={format_count(stats['prompt_tokens'])}",
        f"output={format_count(stats['completion_tokens'])}",
        f"cached={format_count(stats['cached_tokens'])}",
        f"cache_write={format_count(stats['cache_write_input_tokens'])}",
        f"cache_write_1h={format_count(stats['cache_write_1h_input_tokens'])}",
        f"reasoning={format_count(stats['reasoning_tokens'])}",
        f"entries input={format_count(stats['input_entries'])}",
        f"output={format_count(stats['output_entries'])}",
        f"errors={format_count(stats['error_entries'])}",
        f"usage={format_count(stats['usage_entries'])}",
    ]
    model_parts = [
        f"{model} x {count}"
        for model, count in sorted(stats["models"].items())
    ]
    if model_parts:
        parts.append(f"models: {'; '.join(model_parts)}")
    return " | ".join(parts)


def html_pre(text: str) -> str:
    return html.escape(str(text), quote=False)


def render_meta_html(meta: dict, payload: dict) -> str:
    details = []
    model = meta.get("exact_model") or payload.get("exact_model") or payload.get("model", "?")
    instance = meta.get("provider_instance_name")
    for key, value in (
        ("model", model),
        ("instance", instance),
        ("task_id", meta.get("task_id")),
        ("tenant", meta.get("tenant_id")),
        ("caller_app", meta.get("caller_app_id")),
        ("provider", meta.get("provider")),
        ("method", meta.get("method")),
        ("request_model", meta.get("request_model")),
        ("status", meta.get("status") or payload.get("status")),
        ("trace", meta.get("trace_id")),
    ):
        if value and value != "None":
            details.append(
                f"<span><b>{html.escape(key)}</b>: {html.escape(str(value))}</span>"
            )
    return "\n".join(details)


def render_llm_entry_html(index: int, timestamp: str, direction: str, meta: dict, payload: dict, raw: bool) -> str:
    entry_title = "INPUT" if direction == "input" else "OUTPUT"
    css_class = "input" if direction == "input" else "output"
    body = []

    if raw or "_raw" in payload:
        body.append(f"<pre>{html_pre(json.dumps(payload, indent=2, ensure_ascii=False))}</pre>")
    elif direction == "input":
        clean = extract_clean_input(payload)
        for msg in clean.get("messages", []):
            role = msg.get("role", "unknown")
            msg_title = msg.get("title", role.upper())
            content = format_message_content(msg.get("content", ""))
            body.append(
                f"<section class=\"message role-{html.escape(role)}\">"
                f"<h3>{html.escape(msg_title)}</h3>"
                f"<pre>{html_pre(content)}</pre>"
                f"</section>"
            )
        if clean.get("tools"):
            tools = json.dumps(clean["tools"], indent=2, ensure_ascii=False)
            body.append(
                "<section class=\"message role-tool\">"
                "<h3>TOOLS</h3>"
                f"<pre>{html_pre(tools)}</pre>"
                "</section>"
            )
    else:
        clean = extract_clean_output(payload)
        usage = clean.get("usage")
        if usage:
            body.append(
                "<div class=\"usage\">"
                f"tokens: prompt={html.escape(str(usage.get('prompt_tokens')))} "
                f"completion={html.escape(str(usage.get('completion_tokens')))} "
                f"total={html.escape(str(usage.get('total_tokens')))}"
                "</div>"
            )
        for reply in clean.get("replies", []):
            role = reply.get("role", "assistant")
            finish = reply.get("finish_reason", "?")
            content = format_message_content(reply.get("content", ""))
            body.append(
                f"<section class=\"message role-{html.escape(role)}\">"
                f"<h3>{html.escape(role.upper())} <span>finish={html.escape(str(finish))}</span></h3>"
                f"<pre>{html_pre(content)}</pre>"
                f"</section>"
            )
            if reply.get("tool_calls"):
                tool_calls = json.dumps(reply["tool_calls"], indent=2, ensure_ascii=False)
                body.append(f"<pre class=\"tool-calls\">{html_pre(tool_calls)}</pre>")

    return (
        f"<article class=\"entry {css_class}\" id=\"entry-{index}\">"
        f"<details>"
        f"<summary><span class=\"badge\">{entry_title}</span> "
        f"<span class=\"time\">{html.escape(timestamp)}</span></summary>"
        f"<div class=\"meta\">{render_meta_html(meta, payload)}</div>"
        f"{''.join(body)}"
        f"</details>"
        f"</article>"
    )


def render_error_html(index: int, timestamp: str, event: dict) -> str:
    error = format_message_content(event.get("error", event))
    return (
        f"<article class=\"entry error\" id=\"entry-{index}\">"
        f"<details>"
        f"<summary><span class=\"badge\">ERROR</span> <span class=\"time\">{html.escape(timestamp)}</span></summary>"
        f"<div class=\"meta\">{render_meta_html(event, {})}</div>"
        f"<section class=\"message\"><pre class=\"error-text\">{html_pre(error)}</pre></section>"
        f"</details>"
        f"</article>"
    )


def default_html_path(logfile: str) -> Path:
    return Path(logfile).with_name("aicc_log.html")


def render_html_report(logfile: str, entries: list, raw: bool, summary: str) -> str:
    title = "AICC Log"
    stats_text = render_stats_text(collect_html_stats(entries))
    rendered_entries = []
    for index, (entry_type, data) in enumerate(entries, 1):
        if entry_type == "llm":
            timestamp, direction, meta, payload = data
            rendered_entries.append(render_llm_entry_html(index, timestamp, direction, meta, payload, raw))
        elif entry_type == "error":
            timestamp, meta_str = data
            rendered_entries.append(render_error_html(index, timestamp, meta_str))

    return f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{html.escape(title)}</title>
  <style>
    :root {{
      color-scheme: light dark;
      --bg: #f6f7f9;
      --fg: #172033;
      --muted: #667085;
      --panel: #ffffff;
      --border: #d9dee8;
      --input: #e8f5ee;
      --output: #eaf2ff;
      --error: #fff0f0;
      --code: #101828;
    }}
    @media (prefers-color-scheme: dark) {{
      :root {{
        --bg: #111318;
        --fg: #edf0f7;
        --muted: #a1a8b8;
        --panel: #1b1f29;
        --border: #303746;
        --input: #183429;
        --output: #182d4f;
        --error: #4a1f24;
        --code: #f3f6ff;
      }}
    }}
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      background: var(--bg);
      color: var(--fg);
      font: 14px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }}
    header {{
      position: sticky;
      top: 0;
      z-index: 1;
      padding: 8px 22px;
      border-bottom: 1px solid var(--border);
      background: color-mix(in srgb, var(--panel) 90%, transparent);
      backdrop-filter: blur(10px);
    }}
    .summary, .path {{
      color: var(--muted);
      overflow-wrap: anywhere;
      font: 12px/1.35 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    }}
    main {{ max-width: 1280px; margin: 0 auto; padding: 18px; }}
    .entry {{
      margin: 0 0 14px;
      border: 1px solid var(--border);
      border-radius: 8px;
      background: var(--panel);
      overflow: hidden;
    }}
    .entry.input summary {{ background: var(--input); }}
    .entry.output summary {{ background: var(--output); }}
    .entry.error summary {{ background: var(--error); }}
    summary {{
      cursor: pointer;
      padding: 10px 14px;
      font-weight: 700;
    }}
    .badge {{
      display: inline-block;
      min-width: 58px;
      margin-right: 8px;
      font-size: 12px;
      letter-spacing: .04em;
    }}
    .time {{ color: var(--muted); font-weight: 600; }}
    .meta {{
      display: flex;
      flex-wrap: wrap;
      gap: 8px 16px;
      padding: 10px 14px;
      border-bottom: 1px solid var(--border);
      color: var(--muted);
      overflow-wrap: anywhere;
    }}
    .message {{ padding: 12px 14px; border-bottom: 1px solid var(--border); }}
    .message:last-child {{ border-bottom: 0; }}
    h3 {{
      margin: 0 0 8px;
      font-size: 12px;
      letter-spacing: .04em;
      color: var(--muted);
    }}
    h3 span {{ margin-left: 8px; font-weight: 500; }}
    pre {{
      margin: 0;
      white-space: pre-wrap;
      overflow-wrap: anywhere;
      color: var(--code);
      font: 13px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    }}
    .usage {{
      padding: 10px 14px;
      color: var(--muted);
      border-bottom: 1px solid var(--border);
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    }}
    .tool-calls {{ padding: 12px 14px; border-top: 1px solid var(--border); }}
    .error-text {{ color: #d92d20; }}
  </style>
</head>
<body>
  <header>
    <div class="summary">{html.escape(summary)} | {html.escape(stats_text)}</div>
    <div class="path">Original Log: {html.escape(str(Path(logfile).resolve()))}</div>
  </header>
  <main>
    {''.join(rendered_entries)}
  </main>
</body>
</html>
"""


def write_html_report(logfile: str, entries: list, raw: bool, summary: str) -> Path:
    html_path = default_html_path(logfile)
    content = render_html_report(logfile, entries, raw, summary)
    try:
        html_path.write_text(content, encoding="utf-8")
    except OSError:
        fallback = Path.cwd() / html_path.name
        if fallback == html_path:
            raise
        fallback.write_text(content, encoding="utf-8")
        html_path = fallback
    return html_path


def _disable_colors():
    """Clear all color constants for plain text output."""
    global C_RESET, C_BOLD, C_DIM, C_CYAN, C_GREEN, C_YELLOW, C_RED, C_BLUE, C_MAGENTA, C_WHITE, C_GRAY
    C_RESET = C_BOLD = C_DIM = C_CYAN = C_GREEN = C_YELLOW = C_RED = C_BLUE = C_MAGENTA = C_WHITE = C_GRAY = ""
    ROLE_COLORS.update({k: "" for k in ROLE_COLORS})


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    flags = [a for a in sys.argv[1:] if a.startswith("--")]

    raw = "--raw" in flags
    print_mode = "--print" in flags
    show_errors = "--errors" in flags
    if print_mode and ("--no-color" in flags or not sys.stdout.isatty()):
        _disable_colors()

    # Determine log file: explicit arg or auto-detect
    if args:
        logfile = args[0]
    else:
        logfile = find_latest_aicc_log()
        if not logfile:
            root = os.environ.get("BUCKYOS_ROOT", "").strip() or DEFAULT_BUCKYOS_ROOT
            print(f"No aicc log files found under {root}/logs/aicc/")
            print(f"Usage: {sys.argv[0]} [logfile] [--raw] [--errors]")
            print(f"       {sys.argv[0]} [logfile] --print [--raw] [--no-color] [--errors]")
            sys.exit(1)

    entries = parse_entries(logfile, show_errors)

    if not entries:
        print("No aicc.llm.input/output events found" + (" (including errors)." if show_errors else "."))
        print("Request/response logs require an AICC build with structured LLM logging.")
        sys.exit(0)

    summary, _, _ = summarize_entries(entries)

    if print_mode:
        print(f"\n{C_BOLD}{summary}{C_RESET}")
        print(f"{C_DIM}Log file: {logfile}{C_RESET}\n")

        for entry_type, data in entries:
            if entry_type == "llm":
                timestamp, direction, meta, payload = data
                display_entry(timestamp, direction, meta, payload, raw=raw)
            elif entry_type == "error":
                timestamp, meta_str = data
                display_error(timestamp, meta_str)

        print()
    else:
        html_path = write_html_report(logfile, entries, raw, summary)
        print(f"{summary}")
        print(f"Log file: {logfile}")
        print(f"HTML report: {html_path}")


if __name__ == "__main__":
    main()
