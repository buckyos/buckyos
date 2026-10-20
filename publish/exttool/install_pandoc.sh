#!/usr/bin/env bash
# Install the official static pandoc release into
# /opt/buckyos/tools/store/pandoc/ and link /opt/buckyos/tools/bin/pandoc.
#
# pandoc used to be an apt package of the aios worker image (196 MB with its
# data package). It is a tool package of the ExtTool volume now, like the
# other optional CLIs (paios容器需求.md §6.3); the release binary is static
# and embeds its data files, so it runs in every worker that mounts the
# volume. TARGET_ARCH (amd64|aarch64) comes from the Dockerfile.

set -euo pipefail

PANDOC_VERSION=3.12.1
PANDOC_SHA256_AMD64=d0c90410e90204c9ca83b8539fac5c7aed01fd537207e4585849f8abc5df20b8
PANDOC_SHA256_ARM64=445d96fd08801fe636cab1158b4f017ee320ac3b7446328b4fe2f902117b19b9

STORE_DIR=/opt/buckyos/tools/store/pandoc
BIN_DIR=/opt/buckyos/tools/bin

case "${TARGET_ARCH:-}" in
  amd64) asset_arch=amd64; sha256="$PANDOC_SHA256_AMD64" ;;
  aarch64) asset_arch=arm64; sha256="$PANDOC_SHA256_ARM64" ;;
  *) echo "[install_pandoc] unsupported TARGET_ARCH=${TARGET_ARCH:-<unset>}" >&2; exit 1 ;;
esac

url="https://github.com/jgm/pandoc/releases/download/$PANDOC_VERSION/pandoc-$PANDOC_VERSION-linux-$asset_arch.tar.gz"
echo "[install_pandoc] downloading $url"
curl -fL --retry 3 --retry-delay 2 -o /tmp/pandoc.tar.gz "$url"
echo "$sha256  /tmp/pandoc.tar.gz" | sha256sum -c -

# Only the binary (the man pages are of no use in a worker); pandoc-lua and
# pandoc-server are the same executable under other names.
mkdir -p "$STORE_DIR/bin" "$BIN_DIR"
tar -xzf /tmp/pandoc.tar.gz -C "$STORE_DIR/bin" --strip-components=2 "pandoc-$PANDOC_VERSION/bin/pandoc"
rm -f /tmp/pandoc.tar.gz
for name in pandoc pandoc-lua pandoc-server; do
  ln -sfn ../store/pandoc/bin/pandoc "$BIN_DIR/$name"
done

printf '{"bundled": true, "version": "%s", "source": "%s", "sha256": "%s"}\n' \
  "$PANDOC_VERSION" "$url" "$sha256" > "$STORE_DIR/meta.json"

echo "[install_pandoc] installed pandoc $PANDOC_VERSION"
