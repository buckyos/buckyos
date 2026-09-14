#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  configure_jarvis_tunnel.sh [options]

Options:
  --user <user_id>             Owner user id. Default: devtest
  --agent-id <agent_id>        Agent id. Default: buckyos_jarvis
  --bot-token <token>          Telegram Bot API token
  --account-id <id>            Owner Telegram account id, e.g. 5397330802 or user:5397330802
  --bot-account-id <id>        Optional Telegram bot account id
  --zone-did <did>             Optional zone DID. If omitted, read from boot/config
  --session-token <token>      BuckyOS AppClient session token for buckycli
  --control-panel-url <url>    Login endpoint. Default: http://127.0.0.1:3180/kapi/control-panel
  --msg-center-url <url>       Reload endpoint. Default: http://127.0.0.1:3180/kapi/msg-center
  --keep-temp                  Keep temporary files for debugging
  --dry-run                    Print generated JSON paths without writing
  -h, --help                   Show this help

Environment:
  BUCKYCLI                     Override buckycli binary path
  BUCKYOS_ROOT                 BuckyOS root. Default: /opt/buckyos
  BUCKYOS_APPCLIENT_SESSION_TOKEN
  BUCKYOS_CONTROL_PANEL_URL
  BUCKYOS_MSG_CENTER_URL
  JARVIS_TELEGRAM_BOT_TOKEN    Telegram Bot API token
  JARVIS_TELEGRAM_ACCOUNT_ID   Owner Telegram account id
  JARVIS_TELEGRAM_BOT_ACCOUNT_ID
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
USER_ID="devtest"
AGENT_ID="buckyos_jarvis"
BOT_TOKEN="${JARVIS_TELEGRAM_BOT_TOKEN:-}"
ACCOUNT_ID="${JARVIS_TELEGRAM_ACCOUNT_ID:-}"
BOT_ACCOUNT_ID="${JARVIS_TELEGRAM_BOT_ACCOUNT_ID:-}"
SESSION_TOKEN="${BUCKYOS_APPCLIENT_SESSION_TOKEN:-}"
ZONE_DID=""
CONTROL_PANEL_URL="${BUCKYOS_CONTROL_PANEL_URL:-http://127.0.0.1:3180/kapi/control-panel}"
MSG_CENTER_URL="${BUCKYOS_MSG_CENTER_URL:-http://127.0.0.1:3180/kapi/msg-center}"
KEEP_TEMP=0
DRY_RUN=0

if [[ "${EUID:-$(id -u)}" == "0" && -n "${SUDO_USER:-}" ]]; then
  cat >&2 <<'EOF'
Do not run this script with sudo.

buckycli uses the current user's BuckyOS client session. Under sudo it runs as
root and cannot find that session token, which makes system-config access fail.
Run it again as your normal Linux user.
EOF
  exit 2
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --user)
      USER_ID="${2:-}"
      shift 2
      ;;
    --agent-id)
      AGENT_ID="${2:-}"
      shift 2
      ;;
    --bot-token)
      BOT_TOKEN="${2:-}"
      shift 2
      ;;
    --account-id)
      ACCOUNT_ID="${2:-}"
      shift 2
      ;;
    --bot-account-id)
      BOT_ACCOUNT_ID="${2:-}"
      shift 2
      ;;
    --zone-did)
      ZONE_DID="${2:-}"
      shift 2
      ;;
    --session-token)
      SESSION_TOKEN="${2:-}"
      shift 2
      ;;
    --control-panel-url)
      CONTROL_PANEL_URL="${2:-}"
      shift 2
      ;;
    --msg-center-url)
      MSG_CENTER_URL="${2:-}"
      shift 2
      ;;
    --keep-temp)
      KEEP_TEMP=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$USER_ID" || -z "$AGENT_ID" ]]; then
  echo "--user and --agent-id cannot be empty" >&2
  exit 2
fi

resolve_buckycli() {
  if [[ -n "${BUCKYCLI:-}" ]]; then
    echo "$BUCKYCLI"
    return
  fi
  if command -v buckycli >/dev/null 2>&1; then
    command -v buckycli
    return
  fi
  local root="${BUCKYOS_ROOT:-/opt/buckyos}"
  for candidate in \
    "$root/bin/buckycli/buckycli" \
    "$root/bin/buckycli" \
    "$SCRIPT_DIR/rootfs/bin/buckycli/buckycli"
  do
    if [[ -x "$candidate" ]]; then
      echo "$candidate"
      return
    fi
  done
  return 1
}

BUCKYCLI_BIN="$(resolve_buckycli || true)"
if [[ -z "$BUCKYCLI_BIN" ]]; then
  echo "buckycli not found. Set BUCKYCLI or add buckycli to PATH." >&2
  exit 2
fi

if [[ -z "$BOT_TOKEN" ]]; then
  read -r -s -p "Telegram Bot API token: " BOT_TOKEN
  echo
fi

if [[ -z "$ACCOUNT_ID" ]]; then
  read -r -p "Your Telegram account id: " ACCOUNT_ID
fi

if [[ -z "$BOT_TOKEN" || -z "$ACCOUNT_ID" ]]; then
  echo "telegram bot token and account id are required" >&2
  exit 2
fi

login_with_password() {
  local password="$1"
  BUCKYOS_LOGIN_USER="$USER_ID" \
  BUCKYOS_LOGIN_PASSWORD="$password" \
  BUCKYOS_CONTROL_PANEL_URL="$CONTROL_PANEL_URL" \
  python3 - <<'PY'
import base64
import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.request

username = os.environ["BUCKYOS_LOGIN_USER"].strip()
password = os.environ["BUCKYOS_LOGIN_PASSWORD"]
url = os.environ["BUCKYOS_CONTROL_PANEL_URL"].strip()

def b64_sha256(text):
    return base64.b64encode(hashlib.sha256(text.encode("utf-8")).digest()).decode("ascii")

nonce = int(time.time() * 1000)
stored_hash = b64_sha256(f"{password}{username}.buckyos")
password_hash = b64_sha256(f"{stored_hash}{nonce}")
request = {
    "method": "auth.login",
    "params": {
        "username": username,
        "password": password_hash,
        "appid": "buckycli",
        "login_nonce": nonce,
        "remember_me": True,
    },
    "sys": [nonce],
}
data = json.dumps(request).encode("utf-8")
http_request = urllib.request.Request(
    url,
    data=data,
    headers={"Content-Type": "application/json"},
    method="POST",
)
try:
    with urllib.request.urlopen(http_request, timeout=10) as response:
        body = response.read().decode("utf-8")
except urllib.error.HTTPError as error:
    body = error.read().decode("utf-8", errors="replace")
    print(f"auth.login HTTP {error.code}: {body}", file=sys.stderr)
    sys.exit(3)
except Exception as error:
    print(f"auth.login failed: {error}", file=sys.stderr)
    sys.exit(3)

try:
    response = json.loads(body)
except json.JSONDecodeError:
    print(f"auth.login returned non-JSON response: {body}", file=sys.stderr)
    sys.exit(3)

if response.get("error"):
    print(f"auth.login error: {response['error']}", file=sys.stderr)
    sys.exit(3)

result = response.get("result")
if not isinstance(result, dict):
    print(f"auth.login missing result object: {response}", file=sys.stderr)
    sys.exit(3)

session_token = result.get("session_token")
if not isinstance(session_token, str) or not session_token.strip():
    print(f"auth.login missing session_token: {response}", file=sys.stderr)
    sys.exit(3)

print(session_token.strip())
PY
}

if [[ -z "$SESSION_TOKEN" ]]; then
  read -r -s -p "BuckyOS password for $USER_ID: " LOGIN_PASSWORD
  echo
  if [[ -z "$LOGIN_PASSWORD" ]]; then
    echo "password cannot be empty" >&2
    exit 2
  fi
  SESSION_TOKEN="$(login_with_password "$LOGIN_PASSWORD")"
  unset LOGIN_PASSWORD
fi

TMP_DIR="$(mktemp -d)"
if [[ "$KEEP_TEMP" == "1" ]]; then
  echo "Temporary files: $TMP_DIR"
else
  trap 'rm -rf "$TMP_DIR"' EXIT
fi

run_buckycli() {
  BUCKYOS_APPCLIENT_SESSION_TOKEN="$SESSION_TOKEN" \
    SYSTEM_CONSOLE_LOG_LEVEL=error \
    SYSTEM_FILE_LOG_LEVEL=off \
    "$BUCKYCLI_BIN" "$@"
}

get_config() {
  local key="$1"
  local output="$2"
  local raw_output="$output.raw"
  if run_buckycli sys_config --get "$key" >"$raw_output" 2>"$output.err"; then
    if [[ -s "$output.err" ]]; then
      cat "$output.err" >&2
    fi
    if ! python3 - "$key" "$raw_output" "$output" <<'PY'
import json
import pathlib
import sys

key = sys.argv[1]
raw_path = pathlib.Path(sys.argv[2])
out_path = pathlib.Path(sys.argv[3])
text = raw_path.read_text(encoding="utf-8").strip()

if not text:
    print(f"buckycli returned empty config for {key}", file=sys.stderr)
    sys.exit(3)

decoder = json.JSONDecoder()
last_value = None
last_end = None
for index, char in enumerate(text):
    if char not in '{["-0123456789tfn':
        continue
    try:
        value, end = decoder.raw_decode(text[index:])
    except json.JSONDecodeError:
        continue
    tail = text[index + end:].strip()
    if tail and not tail.startswith(("Connect to", "config ", "Warning:", "[")):
        continue
    last_value = value
    last_end = index + end

if last_value is None:
    print(f"buckycli returned non-JSON config for {key}:", file=sys.stderr)
    print(text, file=sys.stderr)
    sys.exit(3)

out_path.write_text(
    json.dumps(last_value, ensure_ascii=False, indent=2) + "\n",
    encoding="utf-8",
)
PY
    then
      return 1
    fi
    return 0
  fi
  cat "$output.err" >&2
  if grep -q "session_token is empty" "$raw_output" "$output.err" 2>/dev/null; then
    echo "buckycli has no session token. Set BUCKYOS_APPCLIENT_SESSION_TOKEN or pass --session-token." >&2
  fi
  return 1
}

set_config_file() {
  local key="$1"
  local input="$2"
  local output="$TMP_DIR/set-${key//\//_}.out"
  if ! run_buckycli sys_config --set_file "$key" "$input" >"$output" 2>&1; then
    cat "$output" >&2
    return 1
  fi
  if grep -qE "config set error:|No write permission|No permission" "$output"; then
    cat "$output" >&2
    echo "failed to write $key" >&2
    return 1
  fi
  echo "$key updated"
}

validate_json_equal() {
  local expected="$1"
  local actual="$2"
  local label="$3"
  python3 - "$expected" "$actual" "$label" <<'PY'
import json
import pathlib
import sys

expected_path = pathlib.Path(sys.argv[1])
actual_path = pathlib.Path(sys.argv[2])
label = sys.argv[3]

try:
    expected = json.loads(expected_path.read_text(encoding="utf-8"))
    actual = json.loads(actual_path.read_text(encoding="utf-8"))
except Exception as error:
    print(f"{label}: failed to parse verification JSON: {error}", file=sys.stderr)
    sys.exit(3)

if actual != expected:
    print(f"{label}: readback does not match the generated config", file=sys.stderr)
    sys.exit(3)

print(f"{label}: verified", file=sys.stderr)
PY
}

reload_msg_center() {
  BUCKYOS_MSG_CENTER_URL="$MSG_CENTER_URL" \
  BUCKYOS_RELOAD_SESSION_TOKEN="$SESSION_TOKEN" \
  python3 - <<'PY'
import json
import os
import sys
import time
import urllib.error
import urllib.request

url = os.environ["BUCKYOS_MSG_CENTER_URL"].strip()
session_token = os.environ["BUCKYOS_RELOAD_SESSION_TOKEN"].strip()
seq = int(time.time() * 1000)
request = {
    "method": "service.reload_settings",
    "params": {},
    "sys": [seq, session_token],
}
http_request = urllib.request.Request(
    url,
    data=json.dumps(request).encode("utf-8"),
    headers={"Content-Type": "application/json"},
    method="POST",
)
try:
    with urllib.request.urlopen(http_request, timeout=20) as response:
        body = response.read().decode("utf-8")
except urllib.error.HTTPError as error:
    body = error.read().decode("utf-8", errors="replace")
    print(f"msg-center reload HTTP {error.code}: {body}", file=sys.stderr)
    sys.exit(3)
except Exception as error:
    print(f"msg-center reload failed: {error}", file=sys.stderr)
    sys.exit(3)

try:
    response = json.loads(body)
except json.JSONDecodeError:
    print(f"msg-center reload returned non-JSON response: {body}", file=sys.stderr)
    sys.exit(3)

if response.get("error"):
    print(f"msg-center reload error: {response['error']}", file=sys.stderr)
    sys.exit(3)

response_sys = response.get("sys")
if not isinstance(response_sys, list) or not response_sys or response_sys[0] != seq:
    print(f"msg-center reload returned a mismatched sequence: {response}", file=sys.stderr)
    sys.exit(3)

result = response.get("result")
if not isinstance(result, dict) or result.get("ok") is not True:
    print(f"msg-center reload returned an invalid result: {response}", file=sys.stderr)
    sys.exit(3)
if result.get("tunnel_started") is not True:
    print(f"msg-center Telegram tunnel did not start: {result}", file=sys.stderr)
    sys.exit(3)

print(
    f"msg-center reloaded: transport={result.get('transport_did')} instance={result.get('tunnel_instance_id')}",
    file=sys.stderr,
)
PY
}

validate_msg_center_settings() {
  local input="$1"
  local label="$2"
  MSG_CENTER_SETTINGS_FILE="$input" \
  MSG_CENTER_SETTINGS_LABEL="$label" \
  python3 - <<'PY'
import json
import os
import pathlib
import sys

settings_path = pathlib.Path(os.environ["MSG_CENTER_SETTINGS_FILE"])
label = os.environ["MSG_CENTER_SETTINGS_LABEL"]
tmp = pathlib.Path(os.environ["TMP_DIR"])
summary_path = tmp / "summary.json"

try:
    settings = json.loads(settings_path.read_text(encoding="utf-8"))
except Exception as error:
    print(f"{label}: failed to parse msg-center settings JSON: {error}", file=sys.stderr)
    sys.exit(3)

try:
    summary = json.loads(summary_path.read_text(encoding="utf-8"))
except Exception as error:
    print(f"{label}: failed to parse generated summary JSON: {error}", file=sys.stderr)
    sys.exit(3)

if not isinstance(settings, dict):
    print(f"{label}: msg-center settings must be a JSON object", file=sys.stderr)
    sys.exit(3)

telegram_tunnel = settings.get("telegram_tunnel")
if not isinstance(telegram_tunnel, dict):
    print(f"{label}: missing top-level telegram_tunnel object", file=sys.stderr)
    sys.exit(3)

if "msg_tunnels" in settings:
    print(f"{label}: unexpected legacy msg_tunnels key; current config must use top-level telegram_tunnel", file=sys.stderr)
    sys.exit(3)

gateway = telegram_tunnel.get("gateway")
if not isinstance(gateway, dict) or gateway.get("mode") != "bot_api":
    print(f"{label}: telegram_tunnel.gateway.mode must be bot_api", file=sys.stderr)
    sys.exit(3)

for legacy_field in ("tunnel_did", "tunnel_id"):
    if legacy_field in telegram_tunnel:
        print(f"{label}: unexpected legacy telegram_tunnel.{legacy_field}", file=sys.stderr)
        sys.exit(3)

expected_transport_did = summary.get("transport_did")
if telegram_tunnel.get("transport_did") != expected_transport_did:
    print(
        f"{label}: telegram_tunnel.transport_did mismatch; expected {expected_transport_did}, got {telegram_tunnel.get('transport_did')}",
        file=sys.stderr,
    )
    sys.exit(3)

expected_tunnel_instance_id = summary.get("tunnel_instance_id")
if telegram_tunnel.get("tunnel_instance_id") != expected_tunnel_instance_id:
    print(
        f"{label}: telegram_tunnel.tunnel_instance_id mismatch; expected {expected_tunnel_instance_id}, got {telegram_tunnel.get('tunnel_instance_id')}",
        file=sys.stderr,
    )
    sys.exit(3)

bindings = telegram_tunnel.get("bindings")
if not isinstance(bindings, list) or not bindings:
    print(f"{label}: telegram_tunnel.bindings must contain at least one binding", file=sys.stderr)
    sys.exit(3)

expected_owner = summary.get("jarvis_did")
binding = None
for item in bindings:
    if isinstance(item, dict) and item.get("owner_did") == expected_owner:
        binding = item
        break

if binding is None:
    print(f"{label}: missing Telegram binding for {expected_owner}", file=sys.stderr)
    sys.exit(3)

bot_token = binding.get("bot_token")
if not isinstance(bot_token, str) or not bot_token.strip():
    print(f"{label}: binding.bot_token must be a non-empty string", file=sys.stderr)
    sys.exit(3)

if "default_chat_id" in binding:
    print(f"{label}: unexpected legacy binding.default_chat_id", file=sys.stderr)
    sys.exit(3)

print(
    f"{label}: verified msg-center Telegram config for {expected_owner} via {expected_transport_did} ({expected_tunnel_instance_id})",
    file=sys.stderr,
)
PY
}

if [[ -z "$ZONE_DID" ]]; then
  get_config "boot/config" "$TMP_DIR/boot.json" || {
    echo "failed to read boot/config. Check buckycli login/session, or pass --zone-did explicitly." >&2
    exit 3
  }
else
  printf '{}\n' >"$TMP_DIR/boot.json"
fi
get_config "services/msg-center/settings" "$TMP_DIR/msg-center-settings.json" || {
  echo "failed to read services/msg-center/settings" >&2
  exit 3
}
get_config "users/$USER_ID/profile" "$TMP_DIR/user-profile.json" || {
  echo "failed to read users/$USER_ID/profile" >&2
  exit 3
}
get_config "agents/$AGENT_ID/doc" "$TMP_DIR/agent-doc.json" || {
  echo "failed to read agents/$AGENT_ID/doc" >&2
  exit 3
}
get_config "agents/$AGENT_ID/settings" "$TMP_DIR/agent-settings.json" || {
  printf '{}\n' >"$TMP_DIR/agent-settings.json"
}

export USER_ID AGENT_ID BOT_TOKEN ACCOUNT_ID BOT_ACCOUNT_ID ZONE_DID TMP_DIR

python3 - <<'PY'
import base64
import json
import os
import pathlib
import re
import sys

tmp = pathlib.Path(os.environ["TMP_DIR"])
user_id = os.environ["USER_ID"]
agent_id = os.environ["AGENT_ID"]
bot_token = os.environ["BOT_TOKEN"].strip()
account_id = os.environ["ACCOUNT_ID"].strip()
bot_account_id = os.environ["BOT_ACCOUNT_ID"].strip()
zone_did = os.environ["ZONE_DID"].strip()

def load_json(name, default):
    path = tmp / name
    if not path.exists():
        return default
    text = path.read_text(encoding="utf-8").strip()
    if not text:
        return default
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        if name == "user-profile.json":
            print(f"failed to parse existing {name}; refusing to overwrite user profile", file=sys.stderr)
            sys.exit(3)
        return default

def save_json(name, value):
    (tmp / name).write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

def extract_document_did(value):
    if isinstance(value, dict):
        for key in ("id", "@id", "did", "zone_did", "zone_id"):
            candidate = value.get(key)
            if isinstance(candidate, str) and candidate.startswith("did:"):
                return candidate
        for key in ("zone_document", "document"):
            candidate = extract_document_did(value.get(key))
            if candidate:
                return candidate
        return None

    if not isinstance(value, str):
        return None
    text = value.strip()
    if text.startswith("did:"):
        return text

    try:
        decoded_json = json.loads(text)
    except json.JSONDecodeError:
        decoded_json = None
    if decoded_json is not None and decoded_json != value:
        candidate = extract_document_did(decoded_json)
        if candidate:
            return candidate

    parts = text.split(".")
    if len(parts) == 3:
        try:
            payload = parts[1] + "=" * (-len(parts[1]) % 4)
            decoded_payload = json.loads(base64.urlsafe_b64decode(payload).decode("utf-8"))
        except (ValueError, UnicodeDecodeError, json.JSONDecodeError):
            return None
        return extract_document_did(decoded_payload)
    return None

def parse_zone_did():
    if zone_did:
        return zone_did
    boot = load_json("boot.json", {})
    resolved = extract_document_did(boot)
    if resolved:
        return resolved
    print("cannot infer zone DID from boot/config.zone_document; pass --zone-did did:web:<zone-host>", file=sys.stderr)
    sys.exit(3)

def split_did(did):
    parts = did.split(":", 2)
    if len(parts) != 3 or parts[0] != "did" or not parts[1] or not parts[2]:
        print(f"invalid zone DID: {did}", file=sys.stderr)
        sys.exit(3)
    return parts[1], parts[2]

def normalize_contact_account(raw):
    value = raw.strip()
    if not value:
        return value
    if value.startswith(("user:", "group:", "channel:")):
        return value
    if re.fullmatch(r"-?\d+", value):
        return "user:" + value
    return value

method, did_id = split_did(parse_zone_did())
agent_doc = load_json("agent-doc.json", {})
jarvis_did = extract_document_did(agent_doc)
if not jarvis_did:
    print(f"cannot read Agent DID from agents/{agent_id}/doc", file=sys.stderr)
    sys.exit(3)
transport_did = f"did:web:tg-tunnel.{did_id}" if method == "web" else "did:bns:msg-center-default-tunnel"
tunnel_instance_id = "tg-main-tunnel"
contact_account_id = normalize_contact_account(account_id)

msg_settings = load_json("msg-center-settings.json", {})
if not isinstance(msg_settings, dict):
    msg_settings = {}
msg_settings.pop("msg_tunnels", None)
telegram_tunnel = msg_settings.get("telegram_tunnel")
if not isinstance(telegram_tunnel, dict):
    telegram_tunnel = {}
telegram_tunnel.pop("tunnel_did", None)
telegram_tunnel.pop("tunnel_id", None)
telegram_tunnel.update({
    "enabled": True,
    "transport_did": transport_did,
    "tunnel_instance_id": tunnel_instance_id,
    "supports_ingress": True,
    "supports_egress": True,
    "gateway": {"mode": "bot_api"},
})
binding = {
    "owner_did": jarvis_did,
    "bot_token": bot_token,
}
if bot_account_id:
    binding["bot_account_id"] = bot_account_id
bindings = [item for item in telegram_tunnel.get("bindings", []) if not (isinstance(item, dict) and item.get("owner_did") == jarvis_did)]
bindings.append(binding)
telegram_tunnel["bindings"] = bindings
msg_settings["telegram_tunnel"] = telegram_tunnel
save_json("msg-center-settings.out.json", msg_settings)

user_profile = load_json("user-profile.json", {})
if not isinstance(user_profile, dict) or not user_profile:
    print(f"users/{user_id}/profile does not exist or is not a JSON object", file=sys.stderr)
    sys.exit(3)
user_did = extract_document_did(user_profile)
if not user_did:
    print(f"users/{user_id}/profile does not contain a valid DID", file=sys.stderr)
    sys.exit(3)
private_extra = user_profile.get("private_extra")
if not isinstance(private_extra, dict):
    private_extra = {}
contact = private_extra.get("system_contact")
if not isinstance(contact, dict):
    contact = {}
contact["did"] = user_did
groups = contact.get("groups") if isinstance(contact.get("groups"), list) else []
tags = contact.get("tags") if isinstance(contact.get("tags"), list) else []
if "users" not in groups:
    groups.append("users")
if "zone_user" not in tags:
    tags.append("zone_user")
contact["groups"] = groups
contact["tags"] = tags
contact_binding = {
    "platform": "telegram",
    "account_id": contact_account_id,
    "display_id": account_id,
    "tunnel_instance_id": tunnel_instance_id,
    "status": "active",
}
contact_bindings = contact.get("bindings") if isinstance(contact.get("bindings"), list) else []
contact_bindings = [item for item in contact_bindings if not (isinstance(item, dict) and item.get("platform") == "telegram")]
contact_bindings.append(contact_binding)
contact["bindings"] = contact_bindings
private_extra["system_contact"] = contact
user_profile["private_extra"] = private_extra
save_json("user-profile.out.json", user_profile)

agent_settings = load_json("agent-settings.json", {})
if not isinstance(agent_settings, dict):
    agent_settings = {}
agent_settings["enabled"] = agent_settings.get("enabled", True)
agent_settings["auto_start"] = agent_settings.get("auto_start", True)
agent_binding = {
    "platform": "telegram",
    "account_id": contact_account_id,
    "display_id": account_id,
    "tunnel_instance_id": tunnel_instance_id,
    "status": "active",
}
agent_bindings = agent_settings.get("bindings") if isinstance(agent_settings.get("bindings"), list) else []
agent_bindings = [item for item in agent_bindings if not (isinstance(item, dict) and item.get("platform") == "telegram")]
agent_bindings.append(agent_binding)
agent_settings["bindings"] = agent_bindings
save_json("agent-settings.out.json", agent_settings)

summary = {
    "zone_did": f"did:{method}:{did_id}",
    "jarvis_did": jarvis_did,
    "transport_did": transport_did,
    "tunnel_instance_id": tunnel_instance_id,
    "user_profile_key": f"users/{user_id}/profile",
    "agent_settings_key": f"agents/{agent_id}/settings",
}
save_json("summary.json", summary)
PY

echo "Generated Jarvis Telegram config:"
cat "$TMP_DIR/summary.json"
validate_msg_center_settings "$TMP_DIR/msg-center-settings.out.json" "generated"

if [[ "$DRY_RUN" == "1" ]]; then
  echo "Dry run output files:"
  echo "  $TMP_DIR/msg-center-settings.out.json"
  echo "  $TMP_DIR/user-profile.out.json"
  echo "  $TMP_DIR/agent-settings.out.json"
  trap - EXIT
  exit 0
fi

set_config_file "users/$USER_ID/profile" "$TMP_DIR/user-profile.out.json"
set_config_file "agents/$AGENT_ID/settings" "$TMP_DIR/agent-settings.out.json"
set_config_file "services/msg-center/settings" "$TMP_DIR/msg-center-settings.out.json"

get_config "services/msg-center/settings" "$TMP_DIR/msg-center-settings.verify.json" || {
  echo "failed to read back services/msg-center/settings after write" >&2
  exit 3
}
validate_msg_center_settings "$TMP_DIR/msg-center-settings.verify.json" "readback"

get_config "users/$USER_ID/profile" "$TMP_DIR/user-profile.verify.json" || {
  echo "failed to read back users/$USER_ID/profile after write" >&2
  exit 3
}
validate_json_equal "$TMP_DIR/user-profile.out.json" "$TMP_DIR/user-profile.verify.json" "user profile readback"

get_config "agents/$AGENT_ID/settings" "$TMP_DIR/agent-settings.verify.json" || {
  echo "failed to read back agents/$AGENT_ID/settings after write" >&2
  exit 3
}
validate_json_equal "$TMP_DIR/agent-settings.out.json" "$TMP_DIR/agent-settings.verify.json" "agent settings readback"

reload_msg_center

echo "Jarvis Telegram config written and msg-center reloaded."
echo "Restart the Jarvis debug process if it is already running."
