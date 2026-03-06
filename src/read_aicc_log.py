#!/usr/bin/env python3
"""
Parse LLM log files and display formatted prompts/responses.

Extracts llm.input and llm.output lines, strips OpenAI protocol noise,
and presents the content in readable, formatted JSON.

Usage:
    python parse_llm_log.py <logfile> [--raw] [--no-color]
    
Options:
    --raw       Show full JSON instead of cleaned-up view
    --no-color  Disable ANSI color codes (for piping to file)
"""

import sys
import json
import re
import textwrap
from datetime import datetime

# ANSI colors (will be cleared if --no-color)
C_RESET = "\033[0m"
C_BOLD = "\033[1m"
C_DIM = "\033[2m"
C_CYAN = "\033[36m"
C_GREEN = "\033[32m"
C_YELLOW = "\033[33m"
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


def parse_log_line(line: str):
    """Parse a single log line, return (timestamp, direction, metadata, json_payload) or None."""
    # Match: 03-05 06:17:55.146 [INFO] aicc.openai.llm.input ...  request={...}
    #    or: 03-05 06:17:55.146 [INFO] aicc.openai.llm.output ... response={...}
    m = re.match(
        r"^(\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+)\s+"  # timestamp
        r"\[(\w+)\]\s+"                                  # level
        r"aicc\.openai\.llm\.(input|output)\s+"          # direction
        r"(.+?)(?:request|response)=(.+)$",              # metadata + json payload
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
        # Try to repair common issues: unescaped quotes in nested JSON strings
        # Use a lenient approach: decode with json.JSONDecoder manually,
        # or fall back to partial extraction
        payload = _try_repair_json(payload_raw)

    return timestamp, direction, meta, payload


def _try_repair_json(raw: str) -> dict:
    """Attempt to repair and parse malformed JSON from logs."""
    # Strategy: try json.loads on progressively shorter prefixes to find valid JSON,
    # or use raw_decode which stops at the first valid object
    decoder = json.JSONDecoder()
    try:
        obj, _ = decoder.raw_decode(raw)
        return obj
    except json.JSONDecodeError:
        pass

    # Last resort: return raw text for display
    return {"_raw": raw}


def _extract_message_content(msg: dict) -> str:
    """Extract text content from a message in either API format.

    Chat Completions API: {"role": "user", "content": "hello"}
    Responses API:        {"role": "user", "content": [{"type": "input_text", "text": "hello"}]}
    """
    content = msg.get("content", "")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        # Responses API: content is a list of typed blocks
        parts = []
        for block in content:
            if isinstance(block, dict):
                # input_text / output_text / text
                text = block.get("text", "")
                if text:
                    parts.append(text)
                # Handle image/audio/other types gracefully
                elif block.get("type"):
                    parts.append(f"[{block['type']}]")
            elif isinstance(block, str):
                parts.append(block)
        return "\n".join(parts)
    return str(content)


def extract_clean_input(payload: dict) -> dict:
    """Extract readable content from an llm.input request payload.

    Supports both:
    - Chat Completions API: payload has "messages" key
    - Responses API: payload has "input" key
    """
    result = {}

    model = payload.get("model")
    if model:
        result["model"] = model

    # Detect API format: "messages" (Chat Completions) vs "input" (Responses API)
    raw_messages = payload.get("messages") or payload.get("input") or []

    clean_messages = []
    for msg in raw_messages:
        role = msg.get("role", "unknown")
        content = _extract_message_content(msg)
        clean_messages.append({"role": role, "content": content})

    if clean_messages:
        result["messages"] = clean_messages

    # Include tools/functions if present
    tools = payload.get("tools") or payload.get("functions")
    if tools:
        result["tools"] = tools

    return result


def extract_clean_output(payload: dict) -> dict:
    """Extract readable content from an llm.output response payload.

    Supports both:
    - Chat Completions API: payload has "choices" with "message"
    - Responses API: payload has "output" with "content"
    """
    result = {}

    model = payload.get("model")
    if model:
        result["model"] = model

    # Normalize usage across both API formats
    usage = payload.get("usage")
    if usage:
        result["usage"] = {
            # Chat Completions uses prompt_tokens/completion_tokens
            # Responses API uses input_tokens/output_tokens
            "prompt_tokens": usage.get("prompt_tokens") or usage.get("input_tokens"),
            "completion_tokens": usage.get("completion_tokens") or usage.get("output_tokens"),
            "total_tokens": usage.get("total_tokens"),
        }

    replies = []

    # --- Chat Completions API format ---
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

    # --- Responses API format ---
    for item in payload.get("output", []):
        if item.get("type") == "message":
            role = item.get("role", "assistant")
            status = item.get("status", "")
            # Extract text from content blocks
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
            # Tool call in Responses API
            entry = {
                "role": "tool_call",
                "content": f"→ {item.get('name', '?')}({json.dumps(item.get('arguments', ''), ensure_ascii=False)})",
                "finish_reason": item.get("status", ""),
            }
            replies.append(entry)

    if replies:
        result["replies"] = replies

    return result


def print_separator(char="─", width=88):
    print(f"{C_DIM}{char * width}{C_RESET}")


def _try_format_xml(text: str) -> str | None:
    """Try to pretty-print XML. Returns formatted string or None."""
    stripped = text.strip()
    if not stripped.startswith("<"):
        return None
    try:
        import xml.dom.minidom as minidom
        dom = minidom.parseString(stripped)
        pretty = dom.toprettyxml(indent="  ")
        # Remove the xml declaration line
        lines = pretty.splitlines()
        if lines and lines[0].startswith("<?xml"):
            lines = lines[1:]
        # Remove blank lines
        return "\n".join(l for l in lines if l.strip())
    except Exception:
        return None


def print_message_content(content: str, role: str, indent: int = 4):
    """Pretty-print a message's content, attempting to parse inner JSON or XML."""
    color = ROLE_COLORS.get(role, C_WHITE)
    prefix = " " * indent

    # Try to parse content as JSON (e.g. assistant replies that are JSON)
    try:
        inner = json.loads(content)
        formatted = json.dumps(inner, indent=2, ensure_ascii=False)
        for line in formatted.splitlines():
            print(f"{prefix}{color}{line}{C_RESET}")
        return
    except (json.JSONDecodeError, TypeError):
        pass

    # Try to parse as XML
    xml_formatted = _try_format_xml(content)
    if xml_formatted:
        for line in xml_formatted.splitlines():
            print(f"{prefix}{color}{line}{C_RESET}")
        return

    # Otherwise print as text, respecting newlines
    for line in content.splitlines():
        print(f"{prefix}{color}{line}{C_RESET}")


def display_entry(timestamp, direction, meta, payload, raw=False):
    """Display one log entry in a readable format."""
    print()
    print_separator("═")

    icon = "📥 INPUT" if direction == "input" else "📤 OUTPUT"
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
                print_separator("·")

        if clean.get("tools"):
            print_separator("·")
            print(f"  {C_BOLD}{C_YELLOW}[TOOLS]{C_RESET}")
            print(f"    {json.dumps(clean['tools'], indent=2, ensure_ascii=False)}")

    elif direction == "output":
        clean = extract_clean_output(payload)

        if clean.get("usage"):
            u = clean["usage"]
            print(f"  {C_DIM}tokens: prompt={u.get('prompt_tokens')} "
                  f"completion={u.get('completion_tokens')} "
                  f"total={u.get('total_tokens')}{C_RESET}")
            print_separator("·")

        for reply in clean.get("replies", []):
            role = reply.get("role", "assistant")
            color = ROLE_COLORS.get(role, C_WHITE)
            print(f"  {C_BOLD}{color}[{role.upper()}]{C_RESET}  "
                  f"{C_DIM}finish={reply.get('finish_reason', '?')}{C_RESET}")
            print_message_content(reply.get("content", ""), role)

            if reply.get("tool_calls"):
                print(f"    {C_YELLOW}tool_calls: "
                      f"{json.dumps(reply['tool_calls'], indent=2, ensure_ascii=False)}{C_RESET}")

    print_separator("═")


def _disable_colors():
    """Clear all color constants for plain text output."""
    global C_RESET, C_BOLD, C_DIM, C_CYAN, C_GREEN, C_YELLOW, C_BLUE, C_MAGENTA, C_WHITE
    C_RESET = C_BOLD = C_DIM = C_CYAN = C_GREEN = C_YELLOW = C_BLUE = C_MAGENTA = C_WHITE = ""
    ROLE_COLORS.update({k: "" for k in ROLE_COLORS})


def main():
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <logfile> [--raw] [--no-color]")
        sys.exit(1)

    logfile = sys.argv[1]
    raw = "--raw" in sys.argv
    if "--no-color" in sys.argv or not sys.stdout.isatty():
        _disable_colors()

    entries = []
    with open(logfile, "r", encoding="utf-8") as f:
        for line in f:
            parsed = parse_log_line(line)
            if parsed:
                entries.append(parsed)

    if not entries:
        print("No llm.input/llm.output entries found.")
        sys.exit(0)

    print(f"\n{C_BOLD}Found {len(entries)} LLM log entries{C_RESET}\n")

    for timestamp, direction, meta, payload in entries:
        display_entry(timestamp, direction, meta, payload, raw=raw)

    print()


if __name__ == "__main__":
    main()