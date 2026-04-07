#!/usr/bin/env python3
"""
Parse LLM log files and display formatted prompts/responses.

Extracts llm.input and llm.output lines, strips OpenAI protocol noise,
and presents the content in readable, formatted JSON.

Usage:
    python read_aicc_log.py [logfile] [--raw] [--no-color] [--errors]

    If no logfile is given, automatically finds the latest aicc log under
    $BUCKYOS_ROOT/logs/aicc/ (defaults to /opt/buckyos/logs/aicc/).

Options:
    --raw       Show full JSON instead of cleaned-up view
    --no-color  Disable ANSI color codes (for piping to file)
    --errors    Also show provider errors (start_failed lines)
"""

import sys
import json
import re
import os
import glob
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
}

DEFAULT_BUCKYOS_ROOT = "/opt/buckyos"


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
    """Parse a single log line, return (timestamp, direction, metadata, json_payload) or None."""
    # Format: 04-07 10:27:30.198 INFO  [openai.rs:926] aicc.openai.llm.input ... request={...}
    #     or: 04-07 10:27:33.474 INFO  [openai.rs:978] aicc.openai.llm.output ... response={...}
    m = re.match(
        r"^(\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+)\s+"   # timestamp
        r"(\w+)\s+"                                       # level (INFO/WARN/etc)
        r"\[\S+\]\s+"                                     # [source:line]
        r"aicc\.openai\.llm\.(input|output)\s+"           # direction
        r"(.+?)(?:request|response)=(.+)$",               # metadata + json payload
        line.strip(),
    )
    if not m:
        return None

    timestamp = m.group(1)
    direction = m.group(3)  # "input" or "output"
    meta_str = m.group(4)
    payload_raw = m.group(5)

    # Parse metadata pairs
    meta = {}
    for kv in re.finditer(r"(\w+)=(\S+)", meta_str):
        meta[kv.group(1)] = kv.group(2)

    try:
        payload = json.loads(payload_raw)
    except json.JSONDecodeError:
        payload = _try_repair_json(payload_raw)

    return timestamp, direction, meta, payload


def parse_error_line(line: str):
    """Parse a provider error line, return (timestamp, metadata_str) or None."""
    # Format: 04-07 10:13:43.364 WARN  [aicc.rs:1561] aicc.provider.start_failed task_id=... err=...
    m = re.match(
        r"^(\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+)\s+"
        r"\w+\s+\[\S+\]\s+"
        r"aicc\.provider\.start_failed\s+"
        r"(.+)$",
        line.strip(),
    )
    if not m:
        return None
    return m.group(1), m.group(2)


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
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for block in content:
            if isinstance(block, dict):
                text = block.get("text", "")
                if text:
                    parts.append(text)
                elif block.get("type"):
                    parts.append(f"[{block['type']}]")
            elif isinstance(block, str):
                parts.append(block)
        return "\n".join(parts)
    return str(content)


def extract_clean_input(payload: dict) -> dict:
    """Extract readable content from an llm.input request payload."""
    result = {}

    model = payload.get("model")
    if model:
        result["model"] = model

    raw_messages = payload.get("messages") or payload.get("input") or []

    clean_messages = []
    for msg in raw_messages:
        role = msg.get("role", "unknown")
        content = _extract_message_content(msg)
        clean_messages.append({"role": role, "content": content})

    if clean_messages:
        result["messages"] = clean_messages

    tools = payload.get("tools") or payload.get("functions")
    if tools:
        result["tools"] = tools

    return result


def extract_clean_output(payload: dict) -> dict:
    """Extract readable content from an llm.output response payload."""
    result = {}

    model = payload.get("model")
    if model:
        result["model"] = model

    usage = payload.get("usage")
    if usage:
        result["usage"] = {
            "prompt_tokens": usage.get("prompt_tokens") or usage.get("input_tokens"),
            "completion_tokens": usage.get("completion_tokens") or usage.get("output_tokens"),
            "total_tokens": usage.get("total_tokens"),
        }

    replies = []

    # Chat Completions API format
    for choice in payload.get("choices", []):
        msg = choice.get("message", {})
        content = msg.get("content", "")
        role = msg.get("role", "assistant")
        entry = {"role": role, "content": content}
        if msg.get("tool_calls"):
            entry["tool_calls"] = msg["tool_calls"]
        if choice.get("finish_reason"):
            entry["finish_reason"] = choice["finish_reason"]
        replies.append(entry)

    # Responses API format
    for item in payload.get("output", []):
        if item.get("type") == "message":
            role = item.get("role", "assistant")
            status = item.get("status", "")
            content_parts = []
            for block in item.get("content", []):
                text = block.get("text", "")
                if text:
                    content_parts.append(text)
                elif block.get("type"):
                    content_parts.append(f"[{block['type']}]")
            content = "\n".join(content_parts)
            entry = {"role": role, "content": content}
            if status:
                entry["finish_reason"] = status
            replies.append(entry)

        elif item.get("type") == "function_call":
            entry = {
                "role": "tool_call",
                "content": f"-> {item.get('name', '?')}({json.dumps(item.get('arguments', ''), ensure_ascii=False)})",
                "finish_reason": item.get("status", ""),
            }
            replies.append(entry)

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

    try:
        inner = json.loads(content)
        formatted = json.dumps(inner, indent=2, ensure_ascii=False)
        for line in formatted.splitlines():
            print(f"{prefix}{color}{line}{C_RESET}")
        return
    except (json.JSONDecodeError, TypeError):
        pass

    xml_formatted = _try_format_xml(content)
    if xml_formatted:
        for line in xml_formatted.splitlines():
            print(f"{prefix}{color}{line}{C_RESET}")
        return

    for line in content.splitlines():
        print(f"{prefix}{color}{line}{C_RESET}")


def display_entry(timestamp, direction, meta, payload, raw=False):
    """Display one log entry in a readable format."""
    print()
    print_separator("=")

    icon = ">> INPUT" if direction == "input" else "<< OUTPUT"
    model = meta.get("model", payload.get("model", "?"))
    instance = meta.get("instance_id", "")
    trace = meta.get("trace_id", "")

    print(f"{C_BOLD}{icon}{C_RESET}  {C_DIM}{timestamp}{C_RESET}  "
          f"{C_YELLOW}{model}{C_RESET}  {C_DIM}{instance}{C_RESET}")

    if trace and trace != "None":
        print(f"  {C_DIM}trace: {trace}{C_RESET}")

    print_separator()

    if raw:
        print(json.dumps(payload, indent=2, ensure_ascii=False))
        return

    if direction == "input":
        clean = extract_clean_input(payload)
        messages = clean.get("messages", [])
        for i, msg in enumerate(messages):
            role = msg["role"]
            color = ROLE_COLORS.get(role, C_WHITE)
            print(f"  {C_BOLD}{color}[{role.upper()}]{C_RESET}")
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


def display_error(timestamp, meta_str):
    """Display a provider error entry."""
    print()
    print_separator("=")
    print(f"{C_BOLD}{C_RED}!! ERROR{C_RESET}  {C_DIM}{timestamp}{C_RESET}")
    print_separator()

    # Parse key=value pairs for readable display
    for kv in re.finditer(r"(\w+)=((?:[^\s]|(?<=\\) )+)", meta_str):
        key, val = kv.group(1), kv.group(2)
        if key == "err":
            # err= runs to end of line
            val = meta_str[meta_str.index("err=") + 4:]
            print(f"  {C_RED}{key}: {val}{C_RESET}")
            break
        else:
            print(f"  {C_DIM}{key}: {val}{C_RESET}")

    print_separator("=")


def _disable_colors():
    """Clear all color constants for plain text output."""
    global C_RESET, C_BOLD, C_DIM, C_CYAN, C_GREEN, C_YELLOW, C_RED, C_BLUE, C_MAGENTA, C_WHITE, C_GRAY
    C_RESET = C_BOLD = C_DIM = C_CYAN = C_GREEN = C_YELLOW = C_RED = C_BLUE = C_MAGENTA = C_WHITE = C_GRAY = ""
    ROLE_COLORS.update({k: "" for k in ROLE_COLORS})


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    flags = [a for a in sys.argv[1:] if a.startswith("--")]

    raw = "--raw" in flags
    show_errors = "--errors" in flags
    if "--no-color" in flags or not sys.stdout.isatty():
        _disable_colors()

    # Determine log file: explicit arg or auto-detect
    if args:
        logfile = args[0]
    else:
        logfile = find_latest_aicc_log()
        if not logfile:
            root = os.environ.get("BUCKYOS_ROOT", "").strip() or DEFAULT_BUCKYOS_ROOT
            print(f"No aicc log files found under {root}/logs/aicc/")
            print(f"Usage: {sys.argv[0]} [logfile] [--raw] [--no-color] [--errors]")
            sys.exit(1)
        print(f"{C_DIM}Auto-detected log: {logfile}{C_RESET}")

    entries = []  # (timestamp, type, data...)
    with open(logfile, "r", encoding="utf-8") as f:
        for line in f:
            parsed = parse_log_line(line)
            if parsed:
                entries.append(("llm", parsed))
                continue
            if show_errors:
                err = parse_error_line(line)
                if err:
                    entries.append(("error", err))

    if not entries:
        print("No llm.input/llm.output entries found.")
        sys.exit(0)

    llm_count = sum(1 for t, _ in entries if t == "llm")
    err_count = sum(1 for t, _ in entries if t == "error")
    summary = f"Found {llm_count} LLM log entries"
    if err_count:
        summary += f", {err_count} errors"

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


if __name__ == "__main__":
    main()
