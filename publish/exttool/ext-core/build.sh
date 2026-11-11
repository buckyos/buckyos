#!/usr/bin/env bash
# Build the payloads of ext-core (doc/pikg/format.md §5.1, ExtTools TODO §3.10) into payload/<platform>/ and, with
# --archive, dist/ext-core-<version>-<platform>.tar.gz plus dist/manifest.json.
#
#   ./build.sh --platform linux-x86_64 --archive
#   buckyos pikg doctor dapp_meta --catalog ..
#   buckyos pikg build dapp_meta && buckyos pikg pack dapp_dist
#   buckyos ext-tools import dapp_dist/ext-core-<version>.pikg     # into a Zone's Repo, digest unchanged
#
# Tools with an official archive are downloaded here, version and SHA256 pinned in the table below (§6.4 of
# doc/app_service/paios容器需求.md: table B says which tool comes from where). The ones without go into a conda-forge
# prefix locked by conda-lock.yml. A launcher in bin/ only ever points into store/ext-core, relative to itself, and never
# falls back to the PATH.
#
# Needs: bash, curl, python3, tar (with xz), unzip, and micromamba (MICROMAMBA, or the one this script downloads).
# darwin-* builds on a Mac: writing the install prefix into a Mach-O breaks its signature, and only macOS can sign it
# again; LibreOffice comes out of its dmg with hdiutil. linux-* builds anywhere. windows-* builds on Windows (msiexec /a
# unpacks LibreOffice's msi) or anywhere with msiextract (msitools).
set -euo pipefail

PANDOC_VERSION=3.12.1
PYTHON_VERSION=3.12.16
PYTHON_RELEASE=20261026
UV_VERSION=0.13.7
NODE_VERSION=v24.21.0
DENO_VERSION=v2.9.2
JQ_VERSION=1.8.2
MINGIT_TAG=v2.56.0.windows.4
MINGIT_VERSION=2.56.0.4
BUSYBOX_VERSION=FRP-6354-g5f09fb446
MICROMAMBA_VERSION=2.10.0-0
LIBREOFFICE_VERSION=26.8.2
NOTO_SANS_CJK_TAG=Sans2.004
NOTO_SERIF_CJK_TAG=Serif2.003

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
manifest="$here/ext-manifest.json"
python_bin="${PYTHON:-python3}"
cache="${EXT_CORE_CACHE:-$here/.cache}"
out="$here/payload"
dist="$here/dist"
platforms=()
archive=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --platform) platforms+=("$2"); shift 2 ;;
    --archive) archive=1; shift ;;
    --out) out="$2"; shift 2 ;;
    --dist) dist="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
if [ "${#platforms[@]}" -eq 0 ]; then
  platforms=(linux-x86_64 linux-aarch64 windows-x86_64)
  [ "$(uname -s)" = Darwin ] && platforms=(darwin-arm64 darwin-x86_64)
fi
mkdir -p "$cache"
version="$("$python_bin" -c 'import json, sys; print(json.load(open(sys.argv[1]))["version"])' "$manifest")"

sha_of() {
  awk -v file="$1" '$2 == file {print $1}' "$here/sources.sha256"
}

sha256_of_file() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

fetch() {
  local url="$1" file="${2:-${1##*/}}" want
  want="$(sha_of "$file")"
  [ -n "$want" ] || { echo "$file is not in sources.sha256" >&2; exit 1; }
  if [ ! -f "$cache/$file" ] || [ "$(sha256_of_file "$cache/$file")" != "$want" ]; then
    if ! curl -fsSL --retry 3 --retry-all-errors --retry-delay 2 --connect-timeout 30 -o "$cache/$file.part" "$url"; then
      rm -f "$cache/$file.part"
      echo "$file: download failed" >&2
      exit 1
    fi
    if [ "$(sha256_of_file "$cache/$file.part")" != "$want" ]; then
      rm -f "$cache/$file.part"
      echo "$file: sha256 differs from sources.sha256" >&2
      exit 1
    fi
    mv "$cache/$file.part" "$cache/$file" || exit 1
  fi
  echo "$cache/$file"
}

conda_platform() {
  case "$1" in
    linux-x86_64) echo linux-64 ;;
    linux-aarch64) echo linux-aarch64 ;;
    darwin-arm64) echo osx-arm64 ;;
    darwin-x86_64) echo osx-64 ;;
    windows-x86_64) echo win-64 ;;
  esac
}

find_micromamba() {
  if [ -n "${MICROMAMBA:-}" ]; then echo "$MICROMAMBA"; return; fi
  if command -v micromamba >/dev/null 2>&1; then command -v micromamba; return; fi
  local dir="$cache/micromamba-$MICROMAMBA_VERSION" host
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) host=linux-64 ;;
    Linux-aarch64) host=linux-aarch64 ;;
    Darwin-arm64) host=osx-arm64 ;;
    Darwin-x86_64) host=osx-64 ;;
    *) echo "no micromamba for this builder" >&2; exit 1 ;;
  esac
  if [ ! -x "$dir/bin/micromamba" ]; then
    mkdir -p "$dir"
    curl -fsSL --retry 3 "https://github.com/mamba-org/micromamba-releases/releases/download/$MICROMAMBA_VERSION/micromamba-$host.tar.bz2" | tar -xj -C "$dir" bin/micromamba >&2
  fi
  echo "$dir/bin/micromamba"
}

# Unix launcher: find ourselves, through any symlink to us, set what the tool needs, run the file in store/.
unix_launcher() {
  local payload="$1" command="$2" target="$3" env_file="${4:-}"
  {
    echo '#!/bin/sh'
    echo 'p="$0"'
    echo 'while [ -h "$p" ]; do l="$(readlink "$p")"; case "$l" in /*) p="$l" ;; *) p="${p%/*}/$l" ;; esac; done'
    echo 'here="$(cd "${p%/*}" && pwd -P)"'
    echo 's="$here/../store/ext-core"'
    [ -n "$env_file" ] && echo ". \"\$s/env/$env_file\""
    echo "exec $target \"\$@\""
  } > "$payload/bin/$command"
  chmod 755 "$payload/bin/$command"
}

# A prefix's data directories are compiled in at their build location, so a relocated tool is told where they are.
unix_env_files() {
  local store="$1/store/ext-core" ghostscript
  mkdir -p "$store/env"
  cat > "$store/env/magick.sh" <<'SH'
c="$s/conda"
export MAGICK_CONFIGURE_PATH="$c/etc/ImageMagick-7:$(echo "$c"/share/ImageMagick-7):$(echo "$c"/lib/ImageMagick-*/config-Q16HDRI)"
export FONTCONFIG_FILE="$c/etc/fonts/fonts.conf"
SH
  cat > "$store/env/tesseract.sh" <<'SH'
export TESSDATA_PREFIX="$s/conda/share/tessdata"
SH
  cat > "$store/env/git.sh" <<'SH'
c="$s/conda"
export GIT_EXEC_PATH="$c/libexec/git-core"
export GIT_TEMPLATE_DIR="$c/share/git-core/templates"
export GIT_SSL_CAINFO="$c/ssl/cacert.pem"
export PATH="$c/bin:$PATH"
SH
  cat > "$store/env/curl.sh" <<'SH'
export CURL_CA_BUNDLE="$s/conda/ssl/cacert.pem"
SH
  cat > "$store/env/fontconfig.sh" <<'SH'
export FONTCONFIG_FILE="$s/conda/etc/fonts/fonts.conf"
SH
  # LibreOffice's own libraries first (its copies of curl, ICU and libxml2 are not the prefix's), then the prefix for what
  # the official build takes from the system. Fonts: the prefix's and LibreOffice's share/fonts, not the system's font
  # directories, so that a conversion looks the same on every machine. Its text handling needs a UTF-8 locale; C.UTF-8 is in every glibc.
  cat > "$store/env/soffice.sh" <<'SH'
export LD_LIBRARY_PATH="$s/libreoffice/program:$s/conda/lib"
export FONTCONFIG_FILE="$s/conda/etc/fonts/fonts.conf"
case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in *[Uu][Tt][Ff]-8|*[Uu][Tt][Ff]8) ;; *) export LC_ALL=C.UTF-8 ;; esac
SH
}

# Windows launcher. %~dp0 ends with a backslash; `exit /b` hands the tool's exit code to the caller.
windows_launcher() {
  local payload="$1" command="$2" target="$3" env_lines="${4:-}"
  {
    printf '@echo off\r\n'
    printf 'setlocal\r\n'
    printf 'set "S=%%~dp0..\\store\\ext-core"\r\n'
    [ -n "$env_lines" ] && printf '%b' "$env_lines"
    printf '%s %%*\r\n' "$target"
    printf 'exit /b %%ERRORLEVEL%%\r\n'
  } > "$payload/bin/$command.cmd"
  chmod 755 "$payload/bin/$command.cmd"
}

# Keep what the commands need of a conda prefix: no headers, static libraries, manuals, GUI toolkits, nor the 126 OCR
# languages outside the table. Symlinks between versioned library names become one file, since the packers read through
# them and every symlink would otherwise be a full copy.
prune_prefix() {
  local prefix="$1" platform="$2"
  "$python_bin" - "$prefix" "$platform" <<'PY'
import os, shutil, sys
from pathlib import Path
prefix = Path(sys.argv[1]); platform = sys.argv[2]
windows = platform.startswith("windows")
top = prefix / "Library" if windows else prefix
def rm(path):
    if path.is_symlink() or path.is_file():
        path.unlink()
    elif path.is_dir():
        shutil.rmtree(path)
for rel in ["include", "man", "share/man", "share/doc", "share/gtk-doc", "share/info", "share/gir-1.0", "share/locale",
            "share/icons", "share/gtk-3.0", "share/themes", "share/aclocal", "share/bash-completion", "share/zsh",
            "share/vala", "share/cmake", "share/glib-2.0/schemas", "share/applications", "share/X11", "share/xkeyboard-config-2",
            "share/cups", "share/terminfo", "share/gdb", "share/gettext", "share/emacs", "share/gitweb", "share/perl5/Git/SVN",
            "lib/cmake", "lib/pkgconfig", "lib/girepository-1.0", "lib/dri", "lib/gtk-3.0", "libexec/at-spi2-registryd",
            "libexec/at-spi-bus-launcher", "libexec/dbus-daemon-launch-helper", "libexec/gio-launch-desktop", "conda-meta",
            "var", "sbin", "etc/conda", "etc/dbus-1", "etc/gtk-3.0", "etc/pulse", "etc/OpenCL", "etc/vpl", "shell"]:
    rm(top / rel) if (top / rel).exists() or (top / rel).is_symlink() else None
lib = top / "lib"
if lib.is_dir():
    for path in list(lib.rglob("*.a")) + list(lib.rglob("*.la")) + list(lib.rglob("*.lib")) + list(lib.glob("openvino-*")):
        rm(path)
    for pattern in ["libgtk-3*", "libgdk-3*", "libgailutil-3*", "libatk-*", "libatk-bridge-*", "libatspi*", "libepoxy*", "libopenvino_*frontend*", "libopenvino_*plugin*", "perl5"]:
        for path in lib.glob(pattern):
            rm(path)
git_core = top / "libexec" / "git-core"
if git_core.is_dir():
    # Built-ins run inside `git`; the dashed names are for scripts of another era, and each would pack as a full copy.
    for path in git_core.iterdir():
        if path.is_symlink() and os.readlink(path) == "git":
            path.unlink()
# OCR language data (§6.4 of doc/app_service/paios容器需求.md): the UI languages, their vertical variants, and the other
# common ones; keep smoke/tesseract.sh and smoke/tesseract.ps1 in step.
tesseract_langs = {"osd", "eng", "chi_sim", "chi_tra", "spa", "fra", "deu", "kor", "jpn", "rus",
                   "chi_sim_vert", "chi_tra_vert", "jpn_vert", "kor_vert",
                   "por", "ita", "nld", "pol", "tur", "ukr", "ara", "fas", "heb", "hin", "ben", "tha", "vie", "ind", "msa"}
for tessdata in (top / "share" / "tessdata", prefix / "share" / "tessdata"):
    if tessdata.is_dir():
        for path in tessdata.iterdir():
            if path.suffix == ".traineddata" and path.stem not in tesseract_langs:
                rm(path)
bin_dir = top / "bin"
keep_bin = {"ffmpeg", "ffprobe", "magick", "convert", "identify", "compare", "pdftotext", "pdftoppm", "pdfinfo", "tesseract",
            "git", "curl", "tree", "unzip", "xz", "gs", "sh"}
if not windows and bin_dir.is_dir():
    for path in bin_dir.iterdir():
        if path.name not in keep_bin and not path.name.startswith("git"):
            rm(path)
if windows:
    scripts = prefix / "Scripts"
    if scripts.is_dir():
        rm(scripts)
    for path in bin_dir.rglob("*"):
        if path.is_file() and path.suffix.lower() not in (".dll", ".exe"):
            rm(path)
    for path in (prefix / "conda-meta",):
        rm(path) if path.exists() else None
else:
    # libfoo.so.1 -> libfoo.so.1.2.3: the shortest versioned name, the one the loader asks for, keeps the bytes; libfoo.so
    # (for linking) goes.
    def is_library(name):
        return ".so" in name or name.endswith(".dylib")
    def is_bare(name):
        return name.endswith(".so") or (name.endswith(".dylib") and name.count(".") == 1)
    groups = {}
    for path in lib.rglob("*") if lib.is_dir() else []:
        if not path.is_symlink() or not is_library(path.name):
            continue
        target = path.resolve()
        if target.is_file() and target.parent == path.parent:
            groups.setdefault(target, []).append(path)
    def mach_o_install_name(path):
        """The name a Mach-O dylib says it goes by (LC_ID_DYLIB): what dyld looks for, whatever the file is called."""
        import struct
        with path.open("rb") as handle:
            data = handle.read(65536)
        if data[:4] != b"\xcf\xfa\xed\xfe":
            return None
        ncmds = struct.unpack_from("<I", data, 16)[0]
        offset = 32
        for _ in range(ncmds):
            cmd, size = struct.unpack_from("<II", data, offset)
            if cmd == 0xd:
                start = offset + struct.unpack_from("<I", data, offset + 8)[0]
                return data[start:offset + size].split(b"\0")[0].decode().rsplit("/", 1)[-1]
            offset += size
        return None
    for target, links in groups.items():
        if target.suffix == ".dylib":
            keep = mach_o_install_name(target)
            if keep is None or (keep != target.name and all(link.name != keep for link in links)):
                continue
            for link in links:
                link.unlink()
            if keep != target.name:
                target.rename(target.parent / keep)
            continue
        versioned = sorted((p for p in links if not is_bare(p.name)), key=lambda p: len(p.name))
        for link in links:
            link.unlink()
        if versioned and not is_bare(target.name):
            target.rename(versioned[0])
    # A package may ship the versioned names as hard links or copies of one library.
    seen = {}
    for path in sorted((p for p in lib.iterdir() if p.is_file() and not p.is_symlink() and p.stat().st_nlink > 1 and ".so" in p.name), key=lambda p: (p.name.endswith(".so"), len(p.name))) if lib.is_dir() else []:
        key = (path.stat().st_dev, path.stat().st_ino)
        if key in seen:
            path.unlink()
        else:
            seen[key] = path
    # Dropping a versioned name can orphan a bare one that pointed at it.
    for path in lib.rglob("*") if lib.is_dir() else []:
        if path.is_symlink() and not path.exists():
            path.unlink()
fonts = top / "etc" / "fonts" / "fonts.conf"
if fonts.is_file():
    import re
    text = fonts.read_text()
    text = re.sub(r"<dir>[^<]*share/fonts</dir>", '<dir prefix="relative">../../share/fonts</dir>', text)
    text = re.sub(r"<dir>[^<]*/fonts</dir>", '<dir prefix="relative">../../fonts</dir>', text)
    text = re.sub(r"<cachedir>[^<]*</cachedir>", "", text)
    text = re.sub(r'<include ignore_missing="yes">[^<]*conf\.d</include>', '<include ignore_missing="yes" prefix="relative">conf.d</include>', text)
    fonts.write_text(text)
PY
}

licenses() {
  local prefix="$1" store="$2"
  mkdir -p "$store/LICENSES"
  "$python_bin" - "$prefix/conda-meta" "$store/LICENSES/conda-forge.tsv" <<'PY'
import json, sys
from pathlib import Path
rows = []
for path in sorted(Path(sys.argv[1]).glob("*.json")):
    meta = json.loads(path.read_text())
    rows.append(f'{meta.get("name")}\t{meta.get("version")}\t{meta.get("license", "")}\t{meta.get("url", "")}')
Path(sys.argv[2]).write_text("name\tversion\tlicense\tsource\n" + "\n".join(rows) + "\n")
PY
}

install_conda() {
  local payload="$1" platform="$2" prefix="$payload/store/ext-core/conda" mm
  mm="$(find_micromamba)"
  MAMBA_ROOT_PREFIX="$cache/mamba" "$mm" create -y -q -p "$prefix" --platform "$(conda_platform "$platform")" -f "$here/conda-lock.yml"
  [ -d "$prefix/conda-meta" ] || { echo "[ext-core] micromamba made no prefix at $prefix" >&2; exit 1; }
  licenses "$prefix" "$payload/store/ext-core"
  prune_prefix "$prefix" "$platform"
  assert_loads "$prefix" "$platform"
}

# What the pruning took must not be what a command loads. Only where this builder can run the loader on the file.
assert_loads() {
  local prefix="$1" platform="$2" missing
  case "$platform:$(uname -s)-$(uname -m)" in
    linux-x86_64:Linux-x86_64|linux-aarch64:Linux-aarch64) ;;
    *) return 0 ;;
  esac
  missing="$(for file in "$prefix"/bin/*; do [ -x "$file" ] && [ ! -L "$file" ] && ldd "$file" 2>/dev/null | grep 'not found' | sed "s|^|$(basename "$file"): |"; done | sort -u || true)"
  if [ -n "$missing" ]; then
    echo "[ext-core] pruned too much:" >&2
    echo "$missing" >&2
    exit 1
  fi
}

install_official() {
  local payload="$1" platform="$2" store="$1/store/ext-core" triple node_plat deno_triple jq_plat pandoc_asset file
  case "$platform" in
    linux-x86_64)   triple=x86_64-unknown-linux-gnu;  node_plat=linux-x64;    jq_plat=linux-amd64; pandoc_asset=linux-amd64.tar.gz ;;
    linux-aarch64)  triple=aarch64-unknown-linux-gnu; node_plat=linux-arm64;  jq_plat=linux-arm64; pandoc_asset=linux-arm64.tar.gz ;;
    darwin-arm64)   triple=aarch64-apple-darwin;      node_plat=darwin-arm64; jq_plat=macos-arm64; pandoc_asset=arm64-macOS.zip ;;
    darwin-x86_64)  triple=x86_64-apple-darwin;       node_plat=darwin-x64;   jq_plat=macos-amd64; pandoc_asset=x86_64-macOS.zip ;;
    windows-x86_64) triple=x86_64-pc-windows-msvc;    node_plat=win-x64;      jq_plat=windows-amd64.exe; pandoc_asset=windows-x86_64.zip ;;
  esac

  file="$(fetch "https://github.com/jgm/pandoc/releases/download/$PANDOC_VERSION/pandoc-$PANDOC_VERSION-$pandoc_asset")"
  mkdir -p "$store/pandoc/bin" "$store/.tmp"
  case "$file" in
    *.tar.gz) tar -xzf "$file" -C "$store/pandoc/bin" --strip-components=2 "pandoc-$PANDOC_VERSION/bin/pandoc" ;;
    *) unzip -q -o "$file" -d "$store/.tmp/pandoc"
       find "$store/.tmp/pandoc" -type f \( -name pandoc -o -name pandoc.exe \) -exec cp {} "$store/pandoc/bin/" \; ;;
  esac
  chmod -f 755 "$store/pandoc/bin/"* || true

  file="$(fetch "https://github.com/astral-sh/python-build-standalone/releases/download/$PYTHON_RELEASE/cpython-$PYTHON_VERSION+$PYTHON_RELEASE-$triple-install_only.tar.gz")"
  tar -xzf "$file" -C "$store"
  rm -rf "$store/python/lib/python3.12/test" "$store/python/lib/python3.12/idlelib" "$store/python/lib/python3.12/tkinter" "$store/python/Lib/test" "$store/python/Lib/idlelib" "$store/python/Lib/tkinter" "$store/python/include" "$store/python/share"
  # The packers read through symlinks: every extra name of these would be another full copy.
  rm -f "$store/python/bin/python" "$store/python/lib/libpython3.12.so"
  [ -e "$store/python/bin/python3.12" ] && mv -f "$store/python/bin/python3.12" "$store/python/bin/python3"
  rm -f "$store/python/lib/"*.a "$store/python/lib/python3.12/config-"*/*.a "$store/python/bin/idle3" "$store/python/bin/idle3.12" "$store/python/bin/2to3" "$store/python/bin/2to3-3.12" "$store/python/bin/pydoc3" "$store/python/bin/pydoc3.12" "$store/python/bin/python3-config" "$store/python/bin/python3.12-config"

  mkdir -p "$store/uv"
  case "$platform" in
    windows-*) file="$(fetch "https://github.com/astral-sh/uv/releases/download/$UV_VERSION/uv-$triple.zip")"
               unzip -q -o "$file" -d "$store/uv" ;;
    *) file="$(fetch "https://github.com/astral-sh/uv/releases/download/$UV_VERSION/uv-$triple.tar.gz")"
       tar -xzf "$file" -C "$store/uv" --strip-components=1 ;;
  esac

  case "$platform" in
    windows-*) file="$(fetch "https://nodejs.org/dist/$NODE_VERSION/node-$NODE_VERSION-$node_plat.zip")"
               unzip -q -o "$file" -d "$store/.tmp/node"
               mv "$store/.tmp/node/node-$NODE_VERSION-$node_plat" "$store/node"
               rm -rf "$store/node/node_modules/corepack" "$store/node/corepack"* ;;
    *) file="$(fetch "https://nodejs.org/dist/$NODE_VERSION/node-$NODE_VERSION-$node_plat.tar.xz")"
       mkdir -p "$store/node"
       tar -xJf "$file" -C "$store/node" --strip-components=1
       rm -rf "$store/node/include" "$store/node/share" "$store/node/lib/node_modules/corepack" "$store/node/bin/corepack" "$store/node/bin/npm" "$store/node/bin/npx" ;;
  esac

  mkdir -p "$store/deno"
  file="$(fetch "https://github.com/denoland/deno/releases/download/$DENO_VERSION/deno-$triple.zip")"
  unzip -q -o "$file" -d "$store/deno"
  chmod -f 755 "$store/deno/"* || true

  mkdir -p "$store/jq"
  file="$(fetch "https://github.com/jqlang/jq/releases/download/jq-$JQ_VERSION/jq-$jq_plat")"
  case "$platform" in
    windows-*) cp "$file" "$store/jq/jq.exe" ;;
    *) cp "$file" "$store/jq/jq"; chmod 755 "$store/jq/jq" ;;
  esac

  case "$platform" in
    windows-*)
      file="$(fetch "https://github.com/git-for-windows/git/releases/download/$MINGIT_TAG/MinGit-$MINGIT_VERSION-64-bit.zip")"
      mkdir -p "$store/mingit"
      unzip -q -o "$file" -d "$store/mingit"
      mkdir -p "$store/busybox"
      file="$(fetch "https://frippery.org/files/busybox/busybox-w64-$BUSYBOX_VERSION.exe")"
      cp "$file" "$store/busybox/busybox.exe"
      ;;
  esac
  rm -rf "$store/.tmp"
}

# LibreOffice, headless conversion only (doc/app_service/paios容器需求.md §6.4 table B): writer, calc, impress, draw, math,
# the import filters and its own fonts. No Python, Java classes, help, gallery, icon themes, extensions, dictionaries,
# database drivers nor GUI media backends. Noto Sans / Serif CJK, regular and bold, go into its font directory on every
# platform: the font set is part of a conversion's fingerprint.
LIBREOFFICE_DEBS="libreoffice{v} libreoffice{v}-ure libobasis{v}-core libobasis{v}-writer libobasis{v}-calc libobasis{v}-impress libobasis{v}-draw libobasis{v}-math libobasis{v}-graphicfilter libobasis{v}-xsltfilter libobasis{v}-ooofonts libobasis{v}-en-us"

install_soffice() {
  local payload="$1" platform="$2" store="$1/store/ext-core" lo="$1/store/ext-core/libreoffice" tmp="$1/store/ext-core/.tmp/libreoffice"
  local base="https://download.documentfoundation.org/libreoffice/stable/$LIBREOFFICE_VERSION" file fonts found
  rm -rf "$tmp" "$lo"
  mkdir -p "$tmp"
  case "$platform" in
    linux-*)
      case "$platform" in
        linux-x86_64) file="$(fetch "$base/deb/x86_64/LibreOffice_${LIBREOFFICE_VERSION}_Linux_x86-64_deb.tar.gz")" ;;
        linux-aarch64) file="$(fetch "$base/deb/aarch64/LibreOffice_${LIBREOFFICE_VERSION}_Linux_aarch64_deb.tar.gz")" ;;
      esac
      tar -xzf "$file" -C "$tmp"
      "$python_bin" - "$tmp" "$tmp/root" "${LIBREOFFICE_VERSION%.*}" $LIBREOFFICE_DEBS <<'PY'
import io, sys, tarfile
from pathlib import Path
src, root, series, names = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3], sys.argv[4:]
def data_of(deb):
    """The data.tar.* member of a .deb, which is an `ar` archive."""
    with deb.open("rb") as handle:
        assert handle.read(8) == b"!<arch>\n", deb
        while True:
            header = handle.read(60)
            if len(header) < 60:
                raise SystemExit(f"{deb}: no data.tar")
            name, size = header[:16].decode().strip().rstrip("/"), int(header[48:58])
            if name.startswith("data.tar"):
                return handle.read(size)
            handle.seek(size + size % 2, 1)
for name in names:
    name = name.replace("{v}", series)
    debs = list(src.glob(f"*/DEBS/{name}_*.deb"))
    if len(debs) != 1:
        raise SystemExit(f"LibreOffice archive: expected one {name}_*.deb, found {len(debs)}")
    with tarfile.open(fileobj=io.BytesIO(data_of(debs[0]))) as tar:
        tar.extractall(root, filter="tar")
PY
      mv "$tmp/root/opt/libreoffice${LIBREOFFICE_VERSION%.*}" "$lo"
      fonts="$lo/share/fonts/truetype"
      # Text that names no language falls back to the Simplified Chinese faces, not to whichever face comes first.
      cat > "$store/conda/etc/fonts/conf.d/64-ext-core-cjk.conf" <<'XML'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <alias><family>sans-serif</family><prefer><family>Noto Sans CJK SC</family></prefer></alias>
  <alias><family>serif</family><prefer><family>Noto Serif CJK SC</family></prefer></alias>
  <alias><family>monospace</family><prefer><family>Noto Sans CJK SC</family></prefer></alias>
</fontconfig>
XML
      ;;
    darwin-*)
      case "$platform" in
        darwin-arm64) file="$(fetch "$base/mac/aarch64/LibreOffice_${LIBREOFFICE_VERSION}_MacOS_aarch64.dmg")" ;;
        darwin-x86_64) file="$(fetch "$base/mac/x86_64/LibreOffice_${LIBREOFFICE_VERSION}_MacOS_x86-64.dmg")" ;;
      esac
      command -v hdiutil >/dev/null 2>&1 || { echo "[ext-core] $platform: LibreOffice's dmg needs hdiutil (build on a Mac)" >&2; exit 1; }
      mkdir -p "$tmp/mnt" "$lo"
      hdiutil attach -nobrowse -readonly -mountpoint "$tmp/mnt" "$file" >/dev/null
      cp -R "$tmp/mnt/LibreOffice.app" "$lo/" || { hdiutil detach "$tmp/mnt" >/dev/null; exit 1; }
      hdiutil detach "$tmp/mnt" >/dev/null
      fonts="$lo/LibreOffice.app/Contents/Resources/fonts/truetype"
      ;;
    windows-*)
      file="$(fetch "$base/win/x86_64/LibreOffice_${LIBREOFFICE_VERSION}_Win_x86-64.msi")"
      mkdir -p "$tmp/msi"
      case "$(uname -s)" in
        MINGW*|MSYS*|CYGWIN*)
          MSYS2_ARG_CONV_EXCL='*' msiexec /a "$(cygpath -w "$file")" /qn "TARGETDIR=$(cygpath -w "$tmp/msi")" ;;
        *)
          command -v msiextract >/dev/null 2>&1 || { echo "[ext-core] $platform: LibreOffice's msi needs msiextract (msitools), or build on Windows" >&2; exit 1; }
          msiextract -C "$tmp/msi" "$file" >/dev/null ;;
      esac
      found="$(find "$tmp/msi" -type f -name soffice.com | head -n 1)"
      [ -n "$found" ] || { echo "[ext-core] $platform: no soffice.com in LibreOffice's msi" >&2; exit 1; }
      mv "$(dirname "$(dirname "$found")")" "$lo"
      # The msi puts LibreOffice's fonts into the Windows font folder; here they go where it loads private fonts from.
      fonts="$lo/share/fonts/truetype"
      mkdir -p "$fonts"
      find "$tmp/msi" -type f \( -iname '*.ttf' -o -iname '*.otf' \) -exec cp {} "$fonts/" \;
      ;;
  esac
  [ -d "$fonts" ] || { echo "[ext-core] $platform: LibreOffice has no font directory at $fonts" >&2; exit 1; }
  prune_soffice "$lo" "$platform"
  for file in NotoSansCJK-Regular.ttc NotoSansCJK-Bold.ttc; do
    cp "$(fetch "https://raw.githubusercontent.com/notofonts/noto-cjk/$NOTO_SANS_CJK_TAG/Sans/OTC/$file")" "$fonts/"
  done
  for file in NotoSerifCJK-Regular.ttc NotoSerifCJK-Bold.ttc; do
    cp "$(fetch "https://raw.githubusercontent.com/notofonts/noto-cjk/$NOTO_SERIF_CJK_TAG/Serif/OTC/$file")" "$fonts/"
  done
  mkdir -p "$store/LICENSES"
  cp "$(fetch "https://raw.githubusercontent.com/notofonts/noto-cjk/$NOTO_SANS_CJK_TAG/LICENSE" NotoCJK-LICENSE)" "$store/LICENSES/NotoCJK-LICENSE"
  found="$(find "$lo" -maxdepth 4 -type f \( -name LICENSE -o -name license.txt \) | head -n 1)"
  [ -n "$found" ] && cp "$found" "$store/LICENSES/LibreOffice-LICENSE"
  rm -rf "$tmp"
  rmdir "$store/.tmp" 2>/dev/null || true
  assert_soffice_loads "$store" "$platform"
}

prune_soffice() {
  "$python_bin" - "$1" "$2" <<'PY'
import shutil, sys
from pathlib import Path
lo, platform = Path(sys.argv[1]), sys.argv[2]
def rm(path):
    if path.is_symlink() or path.is_file():
        path.unlink()
    elif path.is_dir():
        shutil.rmtree(path)
if platform.startswith("darwin"):
    contents = lo / "LibreOffice.app" / "Contents"
    program, share = contents / "Frameworks", contents / "Resources"
    patterns = {
        contents: ["Frameworks/LibreOfficePython.framework", "Resources/python", "Resources/java", "Resources/help", "Resources/gallery",
                   "Resources/extensions/*", "Resources/config/images_*.zip", "Resources/Scripts", "Resources/CREDITS.fodt",
                   "Resources/firebird", "Frameworks/libpyuno*", "Frameworks/libpythonloader*", "Frameworks/libavmedia*",
                   "Frameworks/libfirebird*", "Frameworks/libEngine12*", "Frameworks/libmysqlc*", "Frameworks/libpostgresql*",
                   "Frameworks/libofficebean*", "Library/Spotlight", "Library/QuickLook"],
    }
elif platform.startswith("windows"):
    patterns = {
        lo: ["help", "readmes", "CREDITS.fodt", "LICENSE.html", "share/gallery", "share/extensions/*", "share/Scripts",
             "share/config/images_*.zip", "program/classes", "program/wizards", "program/python-core-*", "program/python*.exe",
             "program/python*.dll", "program/pyuno*", "program/pythonloader*", "program/*.py", "program/firebird*", "program/fbembed*",
             "program/intl", "program/Engine12*", "program/mysqlc*", "program/postgresql-sdbc*", "program/libpq*", "program/avmedia*",
             "program/officebean*", "program/updater*", "program/update_service*", "program/shlxthdl", "program/sbase.exe",
             "program/minidump_upload*", "program/senddoc*", "program/gengal*"],
    }
else:
    patterns = {
        lo: ["help", "readmes", "CREDITS.fodt", "LICENSE.html", "share/gallery", "share/extensions", "share/Scripts",
             "program/classes", "program/wizards", "program/*.py", "program/*.pyi", "program/libavmedia*", "program/liblibreofficekitgtk*",
             "program/libofficebean*", "program/libmysqlclo*", "program/libldapbe2lo*", "program/gengal*", "program/minidump_upload",
             "program/senddoc", "program/gdbtrace"],
    }
for base, globs in patterns.items():
    for pattern in globs:
        for path in list(base.glob(pattern)):
            rm(path)
PY
}

# The official Linux build takes cairo, cups, dbus, fontconfig, freetype, glib, NSS, X11, avahi and Kerberos from the
# system: here they are the prefix's (conda-env.yml). Nothing it loads may be missing from the payload, save glibc.
assert_soffice_loads() {
  local store="$1" platform="$2" missing
  case "$platform:$(uname -s)-$(uname -m)" in
    linux-x86_64:Linux-x86_64|linux-aarch64:Linux-aarch64) ;;
    *) return 0 ;;
  esac
  missing="$(for file in "$store"/libreoffice/program/*.bin "$store"/libreoffice/program/oosplash "$store"/libreoffice/program/*.so*; do
    [ -f "$file" ] && LD_LIBRARY_PATH="$store/libreoffice/program:$store/conda/lib" ldd "$file" 2>/dev/null | grep 'not found' | sed "s|^|$(basename "$file"): |"
  done | sort -u || true)"
  if [ -n "$missing" ]; then
    echo "[ext-core] LibreOffice loads what the payload does not have:" >&2
    echo "$missing" >&2
    exit 1
  fi
}

unix_launchers() {
  local p="$1" platform="$2"
  unix_env_files "$p"
  for c in ffprobe xz tree unzip; do unix_launcher "$p" "$c" "\"\$s/conda/bin/$c\""; done
  unix_launcher "$p" ffmpeg "\"\$s/conda/bin/ffmpeg\"" fontconfig.sh
  for c in magick convert identify compare; do unix_launcher "$p" "$c" "\"\$s/conda/bin/magick\"$( [ "$c" = magick ] || echo " $c")" magick.sh; done
  for c in pdftotext pdftoppm pdfinfo; do unix_launcher "$p" "$c" "\"\$s/conda/bin/$c\"" fontconfig.sh; done
  unix_launcher "$p" tesseract "\"\$s/conda/bin/tesseract\"" tesseract.sh
  unix_launcher "$p" git "\"\$s/conda/bin/git\"" git.sh
  unix_launcher "$p" curl "\"\$s/conda/bin/curl\"" curl.sh
  unix_launcher "$p" pandoc "\"\$s/pandoc/bin/pandoc\""
  unix_launcher "$p" python3 "\"\$s/python/bin/python3\""
  unix_launcher "$p" uv "\"\$s/uv/uv\""
  unix_launcher "$p" node "\"\$s/node/bin/node\""
  unix_launcher "$p" npm "\"\$s/node/bin/node\" \"\$s/node/lib/node_modules/npm/bin/npm-cli.js\""
  unix_launcher "$p" npx "\"\$s/node/bin/node\" \"\$s/node/lib/node_modules/npm/bin/npx-cli.js\""
  unix_launcher "$p" deno "\"\$s/deno/deno\""
  unix_launcher "$p" jq "\"\$s/jq/jq\""
  case "$platform" in
    darwin-*) unix_launcher "$p" soffice "\"\$s/libreoffice/LibreOffice.app/Contents/MacOS/soffice\"" ;;
    *) unix_launcher "$p" soffice "\"\$s/libreoffice/program/soffice\"" soffice.sh ;;
  esac
}

windows_launchers() {
  local p="$1" conda_env='set "C=%S%\\conda"\r\nset "PATH=%C%\\Library\\bin;%C%;%C%\\Library\\mingw-w64\\bin;%PATH%"\r\n'
  local font_env="$conda_env"'set "FONTCONFIG_FILE=%C%\\Library\\etc\\fonts\\fonts.conf"\r\n'
  for c in ffprobe xz curl; do windows_launcher "$p" "$c" "\"%C%\\Library\\bin\\$c.exe\"" "$conda_env"; done
  windows_launcher "$p" ffmpeg "\"%C%\\Library\\bin\\ffmpeg.exe\"" "$font_env"
  local magick_env="$conda_env"'set "MAGICK_CONFIGURE_PATH=%C%\\Library\\bin;%C%\\Library\\etc\\ImageMagick-7;%C%\\Library\\share\\ImageMagick-7"\r\nset "FONTCONFIG_FILE=%C%\\Library\\etc\\fonts\\fonts.conf"\r\n'
  windows_launcher "$p" magick "\"%C%\\Library\\bin\\magick.exe\"" "$magick_env"
  for c in convert identify compare; do windows_launcher "$p" "$c" "\"%C%\\Library\\bin\\magick.exe\" $c" "$magick_env"; done
  for c in pdftotext pdftoppm pdfinfo; do windows_launcher "$p" "$c" "\"%C%\\Library\\bin\\$c.exe\"" "$font_env"; done
  windows_launcher "$p" tesseract "\"%C%\\Library\\bin\\tesseract.exe\"" "$conda_env"'set "TESSDATA_PREFIX=%C%\\share\\tessdata"\r\n'
  windows_launcher "$p" pandoc '"%S%\pandoc\bin\pandoc.exe"'
  windows_launcher "$p" python3 '"%S%\python\python.exe"'
  windows_launcher "$p" uv '"%S%\uv\uv.exe"'
  windows_launcher "$p" node '"%S%\node\node.exe"'
  windows_launcher "$p" npm '"%S%\node\node.exe" "%S%\node\node_modules\npm\bin\npm-cli.js"' 'set "PATH=%S%\\node;%PATH%"\r\n'
  windows_launcher "$p" npx '"%S%\node\node.exe" "%S%\node\node_modules\npm\bin\npx-cli.js"' 'set "PATH=%S%\\node;%PATH%"\r\n'
  windows_launcher "$p" deno '"%S%\deno\deno.exe"'
  windows_launcher "$p" jq '"%S%\jq\jq.exe"'
  windows_launcher "$p" git '"%S%\mingit\cmd\git.exe"'
  mkdir -p "$p/store/ext-core/tree"
  cp "$here/windows/tree.py" "$p/store/ext-core/tree/tree.py"
  windows_launcher "$p" tree '"%S%\python\python.exe" "%S%\tree\tree.py"'
  windows_launcher "$p" unzip '"%S%\busybox\busybox.exe" unzip'
  windows_launcher "$p" soffice '"%S%\libreoffice\program\soffice.com"'
}

for platform in "${platforms[@]}"; do
  payload="$out/$platform"
  rm -rf "$payload"
  mkdir -p "$payload/bin" "$payload/smoke" "$payload/store/ext-core"
  cp "$manifest" "$payload/ext-manifest.json"
  case "$platform" in
    windows-*) cp "$here"/smoke/*.ps1 "$payload/smoke/" ;;
    *) cp "$here"/smoke/*.sh "$payload/smoke/" ;;
  esac
  cp "$here/smoke/soffice_docx.py" "$payload/smoke/"
  install_conda "$payload" "$platform"
  install_official "$payload" "$platform"
  install_soffice "$payload" "$platform"
  case "$platform" in
    windows-*) windows_launchers "$payload" ;;
    *) unix_launchers "$payload" "$platform" ;;
  esac
  echo "[ext-core] $platform: $(du -sh "$payload" | cut -f1) in $payload"
  if [ "$archive" = 1 ]; then
    mkdir -p "$dist"
    tar -czf "$dist/ext-core-$version-$platform.tar.gz" -C "$payload" .
    echo "[ext-core] $dist/ext-core-$version-$platform.tar.gz: $(du -h "$dist/ext-core-$version-$platform.tar.gz" | cut -f1)"
  fi
done

if [ "$archive" = 1 ]; then
  "$python_bin" "$here/release_manifest.py" --version "$version" --dist "$dist"
fi
