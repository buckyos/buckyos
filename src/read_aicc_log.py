#!/usr/bin/env python3
"""
Parse LLM log files and display formatted prompts/responses.

Extracts llm.input and llm.output lines, strips OpenAI protocol noise,
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
    --errors    Also show provider errors (start_failed lines)
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
    """Parse a single log line, return (timestamp, direction, metadata, json_payload) or None."""
    # Format: 04-07 10:27:30.198 INFO  [openai.rs:926] aicc.openai.llm.input ... request={...}
    #     or: 04-07 10:27:33.474 INFO  [openai.rs:978] aicc.openai.llm.output ... response={...}
    m = re.match(
        LOG_PREFIX_RE +
        r"aicc\.(\w+)\.llm\.(input|output)\s+"           # provider + direction
        r"(.+?)(?:request|response)=(.+)$",              # metadata + json payload
        line.strip(),
    )
    if not m:
        return None

    timestamp = m.group(1)
    provider = m.group(3)
    direction = m.group(4)  # "input" or "output"
    meta_str = m.group(5)
    payload_raw = m.group(6)

    meta = parse_kv_pairs(meta_str)
    meta["provider"] = provider

    try:
        payload = json.loads(payload_raw)
    except json.JSONDecodeError:
        payload = _try_repair_json(payload_raw)

    return timestamp, direction, meta, payload


def parse_kv_pairs(text: str) -> dict:
    """Parse log-style key=value pairs, preserving values like Some("jarvis")."""
    meta = {}
    for kv in re.finditer(r"(\w+)=((?:[^\s\"']+|\"[^\"]*\"|'[^']*')+)", text):
        meta[kv.group(1)] = kv.group(2)
    return meta


def parse_context_line(line: str):
    """Parse AICC routing/provider context lines that surround provider LLM logs."""
    stripped = line.strip()

    m = re.match(LOG_PREFIX_RE + r"aicc\.routing output:\s+(.+)$", stripped)
    if m:
        return "routing", parse_kv_pairs(m.group(3))

    m = re.match(LOG_PREFIX_RE + r"aicc\.llm\.input\s+(.+)$", stripped)
    if m:
        return "llm_input", parse_kv_pairs(m.group(3))

    m = re.match(LOG_PREFIX_RE + r"aicc\.provider\.start\s+(.+)$", stripped)
    if m:
        return "provider_start", parse_kv_pairs(m.group(3))

    return None


def provider_key(meta: dict):
    """Build the best available FIFO correlation key for provider request/response logs."""
    return (
        meta.get("instance_id") or meta.get("primary_instance") or "",
        meta.get("model") or meta.get("provider_model") or "",
        meta.get("trace_id") or "",
    )


def merge_meta(base: dict, overlay: dict) -> dict:
    merged = dict(base)
    merged.update(overlay)
    if "model" not in merged and "provider_model" in merged:
        merged["model"] = merged["provider_model"]
    if "instance_id" not in merged and "primary_instance" in merged:
        merged["instance_id"] = merged["primary_instance"]
    return merged


def pop_first(queue_map: dict, key):
    queue = queue_map.get(key)
    if not queue:
        return {}
    item = queue.pop(0)
    if not queue:
        queue_map.pop(key, None)
    return item


def parse_error_line(line: str):
    """Parse a provider error line, return (timestamp, metadata_str) or None."""
    # Format: 04-07 10:13:43.364 WARN  [aicc.rs:1561] aicc.provider.start_failed task_id=... err=...
    m = re.match(
        LOG_PREFIX_RE +
        r"aicc\.provider\.start_failed(?:\.final)?\s+"
        r"(.+)$",
        line.strip(),
    )
    if not m:
        return None
    return m.group(1), m.group(3)


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
    formatted = format_message_content(content)

    for line in formatted.splitlines():
        print(f"{prefix}{color}{line}{C_RESET}")


def format_message_content(content: str) -> str:
    """Format message content for terminal or HTML display."""
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
    model = meta.get("model", payload.get("model", "?"))
    instance = meta.get("instance_id", "")
    trace = meta.get("trace_id", "")

    print(f"{C_BOLD}{icon}{C_RESET}  {C_DIM}{timestamp}{C_RESET}  "
          f"{C_YELLOW}{model}{C_RESET}  {C_DIM}{instance}{C_RESET}")

    details = []
    for key in ("task_id", "tenant", "caller_app", "provider", "status"):
        value = meta.get(key)
        if value and value != "None":
            details.append(f"{key}: {value}")
    if details:
        print(f"  {C_DIM}{'  '.join(details)}{C_RESET}")

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

    meta, tail_key, tail_value = split_error_meta(meta_str)
    for key, val in meta.items():
        print(f"  {C_DIM}{key}: {val}{C_RESET}")

    if tail_key:
        print(f"  {C_RED}{tail_key}: {tail_value}{C_RESET}")

    print_separator("=")


def split_error_meta(meta_str: str):
    tail_key = None
    tail_value = None
    prefix = meta_str
    for key in ("err", "reason"):
        marker = f"{key}="
        pos = meta_str.find(marker)
        if pos >= 0:
            tail_key = key
            tail_value = meta_str[pos + len(marker):]
            prefix = meta_str[:pos].strip()
            break
    return parse_kv_pairs(prefix), tail_key, tail_value


def parse_entries(logfile: str, show_errors: bool):
    entries = []
    task_contexts = {}
    pending_provider = {}
    active_provider = {}

    with open(logfile, "r", encoding="utf-8") as f:
        for line in f:
            context = parse_context_line(line)
            if context:
                kind, meta = context
                task_id = meta.get("task_id")
                if task_id:
                    task_contexts[task_id] = merge_meta(task_contexts.get(task_id, {}), meta)

                if kind == "provider_start":
                    base = task_contexts.get(task_id, {}) if task_id else {}
                    provider_meta = merge_meta(base, meta)
                    pending_provider.setdefault(provider_key(provider_meta), []).append(provider_meta)
                continue

            parsed = parse_log_line(line)
            if parsed:
                timestamp, direction, meta, payload = parsed
                key = provider_key(meta)
                if direction == "input":
                    provider_meta = pop_first(pending_provider, key)
                    meta = merge_meta(provider_meta, meta)
                    active_provider.setdefault(provider_key(meta), []).append(meta)
                else:
                    provider_meta = pop_first(active_provider, key)
                    meta = merge_meta(provider_meta, meta)
                entries.append(("llm", (timestamp, direction, meta, payload)))
                continue
            if show_errors:
                err = parse_error_line(line)
                if err:
                    entries.append(("error", err))

    return entries


def summarize_entries(entries):
    llm_count = sum(1 for t, _ in entries if t == "llm")
    err_count = sum(1 for t, _ in entries if t == "error")
    summary = f"Found {llm_count} LLM log entries"
    if err_count:
        summary += f", {err_count} errors"
    return summary, llm_count, err_count


def html_pre(text: str) -> str:
    return html.escape(str(text), quote=False)


def render_meta_html(meta: dict, payload: dict) -> str:
    details = []
    model = meta.get("model", payload.get("model", "?"))
    instance = meta.get("instance_id", "")
    for key, value in (
        ("model", model),
        ("instance", instance),
        ("task_id", meta.get("task_id")),
        ("tenant", meta.get("tenant")),
        ("caller_app", meta.get("caller_app")),
        ("provider", meta.get("provider")),
        ("status", meta.get("status")),
        ("trace", meta.get("trace_id")),
    ):
        if value and value != "None":
            details.append(
                f"<span><b>{html.escape(key)}</b>: {html.escape(str(value))}</span>"
            )
    return "\n".join(details)


def render_llm_entry_html(index: int, timestamp: str, direction: str, meta: dict, payload: dict, raw: bool) -> str:
    title = "INPUT" if direction == "input" else "OUTPUT"
    css_class = "input" if direction == "input" else "output"
    body = []

    if raw:
        body.append(f"<pre>{html_pre(json.dumps(payload, indent=2, ensure_ascii=False))}</pre>")
    elif direction == "input":
        clean = extract_clean_input(payload)
        for msg in clean.get("messages", []):
            role = msg.get("role", "unknown")
            content = format_message_content(msg.get("content", ""))
            body.append(
                f"<section class=\"message role-{html.escape(role)}\">"
                f"<h3>{html.escape(role.upper())}</h3>"
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
        f"<details open>"
        f"<summary><span class=\"badge\">{title}</span> "
        f"<span class=\"time\">{html.escape(timestamp)}</span></summary>"
        f"<div class=\"meta\">{render_meta_html(meta, payload)}</div>"
        f"{''.join(body)}"
        f"</details>"
        f"</article>"
    )


def render_error_html(index: int, timestamp: str, meta_str: str) -> str:
    meta, tail_key, tail_value = split_error_meta(meta_str)
    rows = []
    for key, value in meta.items():
        rows.append(f"<span><b>{html.escape(key)}</b>: {html.escape(str(value))}</span>")
    if tail_key:
        rows.append(f"<span class=\"error-text\"><b>{html.escape(tail_key)}</b>: {html.escape(str(tail_value))}</span>")
    return (
        f"<article class=\"entry error\" id=\"entry-{index}\">"
        f"<details open>"
        f"<summary><span class=\"badge\">ERROR</span> <span class=\"time\">{html.escape(timestamp)}</span></summary>"
        f"<div class=\"meta\">{''.join(rows)}</div>"
        f"</details>"
        f"</article>"
    )


def default_html_path(logfile: str) -> Path:
    path = Path(logfile)
    if path.suffix == ".log":
        return path.with_suffix(".html")
    return path.with_name(path.name + ".html")


def render_html_report(logfile: str, entries: list, raw: bool, summary: str) -> str:
    title = f"AICC Log - {Path(logfile).name}"
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
      padding: 14px 22px;
      border-bottom: 1px solid var(--border);
      background: color-mix(in srgb, var(--panel) 90%, transparent);
      backdrop-filter: blur(10px);
    }}
    h1 {{ margin: 0 0 4px; font-size: 18px; }}
    .summary, .path {{ color: var(--muted); overflow-wrap: anywhere; }}
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
    <h1>{html.escape(title)}</h1>
    <div class="summary">{html.escape(summary)}</div>
    <div class="path">{html.escape(str(Path(logfile).resolve()))}</div>
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
        print("No llm.input/llm.output entries found.")
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
