FROM ubuntu:24.04

ARG TARGET_ARCH
ARG FREECAD_APPIMAGE_URL=""

LABEL org.opencontainers.image.title="paios/exttool (BuckyOS ExtTool Volume seed)" \
      org.opencontainers.image.description="Seed image for the buckyos-exttool named volume: pre-warmed uv/deno caches + baked tool packages (FreeCAD CLI etc.). Content at /opt/buckyos/tools/ is auto-copied into the empty named volume on first mount." \
      org.opencontainers.image.vendor="BuckyOS DAO"

ENV DEBIAN_FRONTEND=noninteractive

# Minimal toolchain. We only need what is required to warm the caches and
# unpack the tool packages. None of these apt-installed binaries ship with
# the volume payload — only the /opt/buckyos/tools/ tree does.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    file \
    fuse \
    libfuse2t64 \
    unzip \
    xz-utils \
    && rm -rf /var/lib/apt/lists/*

# uv — used here only to populate /opt/buckyos/tools/uv-cache with the most
# common Python deps so Script apps can run offline on first start.
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
ENV PATH="/root/.local/bin:${PATH}"

# Layout: §9 of paios容器需求.md.
#   store/        — tool packages, each self-contained (tool/<name>/...)
#   bin/          — per-tool launcher symlinks, added to PATH by entrypoint
#   uv-cache/     — pre-warmed uv package cache (OverlayFS lower for apps)
#   deno-cache/   — pre-warmed Deno module cache  (OverlayFS lower for apps)
RUN mkdir -p \
    /opt/buckyos/tools/store \
    /opt/buckyos/tools/bin \
    /opt/buckyos/tools/uv-cache \
    /opt/buckyos/tools/deno-cache

ENV UV_CACHE_DIR=/opt/buckyos/tools/uv-cache \
    DENO_DIR=/opt/buckyos/tools/deno-cache

# --- Warm the uv cache -----------------------------------------------------
# We do NOT install the Python interpreter here; we only populate the
# global uv download/metadata cache so `uv pip install -r requirements.txt`
# runs offline on first app start.
RUN uv cache dir && \
    uv python install 3.12 && \
    uv pip install --python $(uv python find 3.12) --system \
        --target /tmp/uv-warm \
        requests httpx pydantic pyyaml typer rich \
    && rm -rf /tmp/uv-warm

# --- Warm the deno cache ---------------------------------------------------
# Deno ships into the ExtTool image only to drive `deno cache`; it does NOT
# land in the volume — only its module cache does.
RUN curl -fsSL https://deno.land/install.sh | DENO_INSTALL=/usr/local sh && \
    printf 'import "jsr:@std/path";\nimport "jsr:@std/fs";\nimport "jsr:@std/fmt/colors";\nimport "jsr:@std/json";\n' > /tmp/warm.ts && \
    deno cache /tmp/warm.ts && \
    rm /tmp/warm.ts

# --- FreeCAD CLI tool package ---------------------------------------------
# FreeCAD ships as a self-contained package under tools/store/freecad/ with
# a launcher symlinked from tools/bin/. We use an AppImage (self-contained,
# includes Qt/Python/libs) so the Worker image — which does NOT have Qt —
# can still invoke freecadcmd.
#
# FREECAD_APPIMAGE_URL is arch-specific; build_exttool selects the right
# URL. If unset (e.g. arch without an upstream AppImage), we install a
# stub wrapper that fails loudly so callers know the tool is missing.
COPY install_freecad.sh /tmp/install_freecad.sh
RUN chmod 755 /tmp/install_freecad.sh && \
    FREECAD_APPIMAGE_URL="${FREECAD_APPIMAGE_URL}" /tmp/install_freecad.sh && \
    rm -f /tmp/install_freecad.sh

# Minimal seed entrypoint. The image is run by node_daemon as
#   docker run --rm -v buckyos-exttool:/opt/buckyos/tools paios/exttool:latest
# which auto-copies the image's /opt/buckyos/tools content into the empty
# named volume. The "true" command exits cleanly once that happens.
WORKDIR /opt/buckyos/tools
CMD ["true"]
