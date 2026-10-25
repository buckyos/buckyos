#!/bin/sh
# Download, verify and run a released BuckyOS Agent Daemon on Linux.
# Releases: https://github.com/buckyos/buckyos/releases/tag/agent-daemon-latest
set -eu
umask 077

github=https://github.com/buckyos/buckyos/releases/download

usage() {
    printf '%s\n' 'Usage: start-buckyos-agent-daemon.sh [--download-only] [--artifact FILE --sha256 HASH] [daemon arguments or access URL]'
    printf '%s\n' 'Environment: BUCKYOS_AGENT_VERSION (default: version named by latest.json),'
    printf '%s\n' '             BUCKYOS_AGENT_RELEASE_URL (HTTPS directory holding latest.json and the archives),'
    printf '%s\n' '             BUCKYOS_AGENT_CACHE_DIR'
}

fetch() {
    curl --fail --silent --show-error --location --retry 3 --proto '=https' --tlsv1.2 "$1" --output "$2"
}

valid_version() {
    case "$1" in ''|*[!A-Za-z0-9._+-]*) return 1 ;; esac
}

download_only=false
artifact=
digest=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --download-only) download_only=true; shift ;;
        --artifact) [ "$#" -ge 2 ] || exit 2; artifact=$2; shift 2 ;;
        --sha256) [ "$#" -ge 2 ] || exit 2; digest=$2; shift 2 ;;
        --help|-h) usage; exit 0 ;;
        *) break ;;
    esac
done
if [ "$#" -eq 0 ] && [ "$download_only" = false ]; then usage; exit 2; fi
[ "$(uname -s)" = Linux ] || { printf '%s\n' 'This launcher supports Linux only; macOS and Windows users unpack the release archive directly.' >&2; exit 2; }
case "$(uname -m)" in
    x86_64) target=x86_64-unknown-linux-musl ;;
    aarch64|arm64) target=aarch64-unknown-linux-gnu ;;
    *) printf '%s\n' 'Unsupported Linux architecture.' >&2; exit 2 ;;
esac
cache=${BUCKYOS_AGENT_CACHE_DIR:-${XDG_CACHE_HOME:-${HOME:?}/.cache}/buckyos-agent-daemon}
mkdir -p "$cache"
chmod 700 "$cache"
temporary=$(mktemp -d "$cache/.download.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
version=${BUCKYOS_AGENT_VERSION:-}
if [ -n "$artifact" ]; then
    [ -n "$digest" ] || { printf '%s\n' '--artifact requires --sha256.' >&2; exit 2; }
    cp "$artifact" "$temporary/archive.tar.gz"
else
    if [ -z "$version" ]; then
        fetch "${BUCKYOS_AGENT_RELEASE_URL:-$github/agent-daemon-latest}/latest.json" "$temporary/latest.json"
        version=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([A-Za-z0-9._+-]*\)".*/\1/p' "$temporary/latest.json" | head -n 1)
        valid_version "$version" || { printf '%s\n' 'latest.json does not name a valid version.' >&2; exit 1; }
    fi
    valid_version "$version" || { printf '%s\n' 'Invalid version.' >&2; exit 2; }
    release=${BUCKYOS_AGENT_RELEASE_URL:-$github/agent-daemon-v$version}
    case "$release" in https://*) ;; *) printf '%s\n' 'Release URL must use HTTPS.' >&2; exit 2 ;; esac
    name=buckyos-agent-daemon-$version-$target.tar.gz
    fetch "$release/$name" "$temporary/archive.tar.gz"
    if [ -z "$digest" ]; then
        fetch "$release/$name.sha256" "$temporary/checksum"
        digest=$(cut -d ' ' -f 1 < "$temporary/checksum")
    fi
fi
case "$digest" in ''|*[!0-9a-fA-F]*) printf '%s\n' 'Invalid SHA-256 digest.' >&2; exit 2 ;; esac
[ "${#digest}" -eq 64 ] || { printf '%s\n' 'Invalid SHA-256 digest length.' >&2; exit 2; }
printf '%s  %s\n' "$digest" "$temporary/archive.tar.gz" | sha256sum --check --status || { printf '%s\n' 'Artifact checksum mismatch.' >&2; exit 1; }
# The archive holds one directory buckyos-agent-daemon-<version>-<target>/ with the files below.
mkdir "$temporary/unpack"
tar --extract --gzip --file "$temporary/archive.tar.gz" --directory "$temporary/unpack" --strip-components=1 --no-same-owner --no-same-permissions
for file in buckyos_agent_daemon manifest.json service.sh; do
    [ -f "$temporary/unpack/$file" ] && [ ! -L "$temporary/unpack/$file" ] || { printf '%s\n' "Archive lacks $file." >&2; exit 1; }
done
manifest_version=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([A-Za-z0-9._+-]*\)".*/\1/p' "$temporary/unpack/manifest.json" | head -n 1)
valid_version "$manifest_version" || { printf '%s\n' 'manifest.json does not name a valid version.' >&2; exit 1; }
if [ -n "$version" ] && [ "$version" != "$manifest_version" ]; then
    printf '%s\n' "Archive version $manifest_version does not match requested $version." >&2; exit 1
fi
version=$manifest_version
chmod 700 "$temporary/unpack/buckyos_agent_daemon" "$temporary/unpack/service.sh"
destination=$cache/$version-$target-$digest
if [ ! -d "$destination" ]; then
    mv "$temporary/unpack" "$destination"
fi
rm -rf "$temporary"
trap - EXIT HUP INT TERM
if [ "$download_only" = true ]; then printf '%s\n' "$destination/buckyos_agent_daemon"; exit 0; fi
case "$1" in wss://*|ws://*) access_url=$1; shift; set -- run --url "$access_url" "$@" ;; esac
exec "$destination/buckyos_agent_daemon" "$@"
