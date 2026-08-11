#!/usr/bin/env bash
# Fetch + extract a FreeCAD AppImage into /opt/buckyos/tools/store/freecad/
# and register /opt/buckyos/tools/bin/freecadcmd as the launcher.
#
# FREECAD_APPIMAGE_URL is set by build_exttool per target arch. When unset
# (arch without a prebuilt AppImage), install a loud stub so callers find
# out at call time instead of silently picking up system freecadcmd.

set -euo pipefail

STORE_DIR=/opt/buckyos/tools/store/freecad
BIN_DIR=/opt/buckyos/tools/bin

mkdir -p "$STORE_DIR" "$BIN_DIR"

if [[ -z "${FREECAD_APPIMAGE_URL:-}" ]]; then
  cat > "$BIN_DIR/freecadcmd" <<'STUB'
#!/usr/bin/env bash
echo "[freecadcmd] not bundled for this architecture in paios/exttool" >&2
echo "[freecadcmd] set FREECAD_APPIMAGE_URL at build time to enable" >&2
exit 127
STUB
  chmod 755 "$BIN_DIR/freecadcmd"
  printf '{"bundled": false, "reason": "no FREECAD_APPIMAGE_URL at build time"}\n' > "$STORE_DIR/meta.json"
  exit 0
fi

echo "[install_freecad] downloading $FREECAD_APPIMAGE_URL"
curl -fL --retry 3 --retry-delay 2 -o /tmp/freecad.AppImage "$FREECAD_APPIMAGE_URL"
chmod +x /tmp/freecad.AppImage

echo "[install_freecad] extracting AppImage"
(
  cd /tmp
  /tmp/freecad.AppImage --appimage-extract >/dev/null
)

# Move the extracted tree into the tool store. `squashfs-root` is the
# canonical AppImage extraction root.
rm -rf "$STORE_DIR/squashfs-root"
mv /tmp/squashfs-root "$STORE_DIR/squashfs-root"
rm -f /tmp/freecad.AppImage

# Pick the CLI launcher. FreeCAD AppImages ship `AppRun` at the root and
# `usr/bin/freecadcmd` inside. Prefer the explicit CLI binary.
LAUNCHER=""
for candidate in \
    "$STORE_DIR/squashfs-root/usr/bin/freecadcmd" \
    "$STORE_DIR/squashfs-root/usr/bin/FreeCADCmd" \
    "$STORE_DIR/squashfs-root/AppRun"; do
  if [[ -x "$candidate" ]]; then
    LAUNCHER="$candidate"
    break
  fi
done

if [[ -z "$LAUNCHER" ]]; then
  echo "[install_freecad] no FreeCAD CLI launcher found under $STORE_DIR" >&2
  exit 1
fi

# Wrapper script so PATH entries don't leak the full squashfs-root path into
# users' environments and so we can tweak library paths later if needed.
cat > "$BIN_DIR/freecadcmd" <<WRAPPER
#!/usr/bin/env bash
exec "$LAUNCHER" "\$@"
WRAPPER
chmod 755 "$BIN_DIR/freecadcmd"

printf '{"bundled": true, "source": "%s", "launcher": "%s"}\n' \
  "$FREECAD_APPIMAGE_URL" "$LAUNCHER" > "$STORE_DIR/meta.json"

echo "[install_freecad] installed; launcher=$LAUNCHER"
