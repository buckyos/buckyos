FROM ubuntu:24.04 AS builder

ARG TARGET_ARCH
ARG FREECAD_APPIMAGE_URL=""

ENV DEBIAN_FRONTEND=noninteractive

# Build-only toolchain: used to warm the caches and unpack the tool packages.
# Only the /opt/buckyos/tools/ tree reaches the final image.
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
# The interpreter stays in the builder; only the global uv download/metadata
# cache ships, so `uv pip install -r requirements.txt` runs offline on first
# app start.
RUN uv cache dir && \
    uv python install 3.12 && \
    uv pip install --python $(uv python find 3.12) --system \
        --target /tmp/uv-warm \
        requests httpx pydantic pyyaml typer rich \
    && rm -rf /tmp/uv-warm

# --- Warm the deno cache ---------------------------------------------------
# Deno only drives `deno cache` here; its module cache ships, the binary does not.
RUN curl -fsSL https://deno.land/install.sh | DENO_INSTALL=/usr/local sh && \
    printf 'import "jsr:@std/path";\nimport "jsr:@std/fs";\nimport "jsr:@std/fmt/colors";\nimport "jsr:@std/json";\n' > /tmp/warm.ts && \
    deno cache /tmp/warm.ts && \
    rm /tmp/warm.ts

# --- FreeCAD CLI tool package (optional) -----------------------------------
# FreeCAD is not part of the default image (it was ~90% of its size). With
# FREECAD_APPIMAGE_URL set, build_exttool bakes the AppImage under
# tools/store/freecad/ with a launcher in tools/bin/; otherwise a stub
# freecadcmd explains how to build an image that includes it.
COPY install_freecad.sh /tmp/install_freecad.sh
RUN chmod 755 /tmp/install_freecad.sh && \
    FREECAD_APPIMAGE_URL="${FREECAD_APPIMAGE_URL}" /tmp/install_freecad.sh && \
    rm -f /tmp/install_freecad.sh

# Seed image: only the volume payload plus busybox, whose sh/true/touch the
# node_daemon seeder needs. node_daemon runs it once per image id as
#   docker run --rm -v buckyos-exttool-<id12>:/opt/buckyos/tools --entrypoint /bin/sh \
#     <image> -c 'touch /opt/buckyos/tools/.buckyos-seeded-<id12>'
# Docker copies /opt/buckyos/tools into the empty named volume before the
# command runs, so the marker proves the copy completed.
FROM busybox:1.36.1

COPY --from=builder /opt/buckyos/tools /opt/buckyos/tools

ARG IMAGE_VERSION
ARG SOURCE_REVISION
ARG BUILD_CREATED
LABEL org.opencontainers.image.title="paios/exttool (BuckyOS ExtTool Volume seed)" \
      org.opencontainers.image.description="Seed image for the buckyos-exttool-<id> named volume: pre-warmed uv/deno caches + optional baked tool packages. Content at /opt/buckyos/tools/ is auto-copied into the empty named volume on first mount." \
      org.opencontainers.image.vendor="BuckyOS DAO" \
      org.opencontainers.image.version="${IMAGE_VERSION}" \
      org.opencontainers.image.revision="${SOURCE_REVISION}" \
      org.opencontainers.image.created="${BUILD_CREATED}"

WORKDIR /opt/buckyos/tools
CMD ["true"]
