#!/bin/sh
set -eu
umask 077

usage() {
    printf '%s\n' 'Usage: service.sh install --binary FILE --sha256 HASH --url-file FILE'
    printf '%s\n' '       service.sh upgrade --binary FILE --sha256 HASH'
    printf '%s\n' '       service.sh stop|uninstall'
    printf '%s\n' 'Explicit Linux user service. Saves the URL privately; never enables lingering or automatic restart.'
}

[ "$(uname -s)" = Linux ] || exit 2
[ "$#" -ge 1 ] || { usage; exit 2; }
action=$1; shift
case "$action" in install|upgrade|stop|uninstall) ;; *) usage; exit 2 ;; esac
binary= digest= url_file=
while [ "$#" -gt 0 ]; do
    [ "$#" -ge 2 ] || { usage; exit 2; }
    case "$1" in
        --binary) binary=$2 ;;
        --sha256) digest=$2 ;;
        --url-file) url_file=$2 ;;
        *) usage; exit 2 ;;
    esac
    shift 2
done
systemctl --user show-environment >/dev/null
config=${XDG_CONFIG_HOME:-${HOME:?}/.config}/buckyos-agent-daemon
units=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
product=$HOME/.local/lib/buckyos-agent-daemon
data=${XDG_DATA_HOME:-$HOME/.local/share}/buckyos-agent-daemon
unit=buckyos-agent-daemon.service

if [ "$action" = stop ]; then systemctl --user stop "$unit"; exit 0; fi
if [ "$action" = uninstall ]; then
    if [ -f "$units/$unit" ]; then systemctl --user disable --now "$unit"; fi
    rm -f "$units/$unit" "$config/access-url" "$product/buckyos_agent_daemon" "$product/buckyos_agent_daemon.previous"
    systemctl --user daemon-reload
    printf '%s\n' 'User service and saved URL removed. Executor ledger and user files retained.'
    exit 0
fi
[ -f "$binary" ] && [ -n "$digest" ] || { usage; exit 2; }
case "$digest" in *[!0-9a-fA-F]*) exit 2 ;; esac
[ "${#digest}" -eq 64 ] || exit 2
actual=$(sha256sum < "$binary" | cut -d ' ' -f 1)
[ "$actual" = "$digest" ] || { printf '%s\n' 'Binary checksum mismatch.' >&2; exit 1; }
if [ "$action" = install ]; then
    [ -f "$url_file" ] && [ ! -L "$url_file" ] || { printf '%s\n' 'A private regular URL file is required.' >&2; exit 2; }
    [ "$(stat -c %u "$url_file")" = "$(id -u)" ] && [ "$(stat -c %a "$url_file")" = 600 ] || {
        printf '%s\n' 'URL file must belong to the current user with mode 0600.' >&2; exit 2;
    }
else
    [ -f "$units/$unit" ] && [ -f "$config/access-url" ] || { printf '%s\n' 'Install the user service before upgrading.' >&2; exit 2; }
fi
mkdir -p "$config" "$units" "$product" "$data"
chmod 700 "$config" "$product" "$data"
temporary=$(mktemp "$product/.binary.XXXXXX")
trap 'rm -f "$temporary"' EXIT HUP INT TERM
cp "$binary" "$temporary"
chmod 700 "$temporary"
"$temporary" --version >/dev/null
if [ -f "$units/$unit" ]; then systemctl --user stop "$unit"; fi
if [ -f "$product/buckyos_agent_daemon" ]; then
    cp "$product/buckyos_agent_daemon" "$product/buckyos_agent_daemon.previous"
fi
mv "$temporary" "$product/buckyos_agent_daemon"
trap - EXIT HUP INT TERM
if [ "$action" = install ]; then
    cp "$url_file" "$config/access-url"
    chmod 600 "$config/access-url"
fi
escape() {
    case "$1" in *'
'*) printf '%s\n' 'Newlines in installation paths are unsupported.' >&2; exit 2 ;; esac
    printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g; s/%/%%/g; s/\$/$$/g'
}
cat > "$units/$unit" <<EOF
[Unit]
Description=BuckyOS Agent Daemon (explicit user installation)
After=network-online.target

[Service]
Type=simple
ExecStart="$(escape "$product/buckyos_agent_daemon")" --data-dir "$(escape "$data")" run --url-file "$(escape "$config/access-url")"
Restart=no
TimeoutStopSec=10
KillMode=mixed
UMask=0077

[Install]
WantedBy=default.target
EOF
systemctl --user daemon-reload
systemctl --user enable --now "$unit"
sleep 2
if ! systemctl --user is-active --quiet "$unit"; then
    if [ "$action" = upgrade ] && [ -f "$product/buckyos_agent_daemon.previous" ]; then
        systemctl --user stop "$unit"
        mv "$product/buckyos_agent_daemon.previous" "$product/buckyos_agent_daemon"
        systemctl --user start "$unit"
        printf '%s\n' 'Upgrade did not stay active; restored the previous binary. Check doctor and URL validity.' >&2
    else
        printf '%s\n' 'Service exited. Check doctor and URL validity; automatic restart is disabled.' >&2
    fi
    exit 1
fi
printf '%s\n' 'User service running. URL expiry or revocation stops it without restart.'
