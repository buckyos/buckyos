FROM ubuntu:24.04

ARG TARGET_ARCH

LABEL org.opencontainers.image.title="paios/aios (BuckyOS Worker Image)" \
      org.opencontainers.image.description="Unified worker runtime for BuckyOS apps and OpenDAN agents" \
      org.opencontainers.image.vendor="BuckyOS DAO"

ENV DEBIAN_FRONTEND=noninteractive

# Runtime + predefined worker CLI tools. No compile toolchain: the Crafter
# image (deferred per requirements §7.8) is the only place build-time deps
# are allowed to land.
RUN apt-get update && apt-get install -y --no-install-recommends \
    bash \
    ca-certificates \
    curl \
    git \
    jq \
    locales \
    rsync \
    tmux \
    tree \
    unzip \
    xz-utils \
    python3 \
    python3-venv \
    nodejs \
    npm \
    ffmpeg \
    imagemagick \
    poppler-utils \
    pandoc \
    tesseract-ocr \
    && rm -rf /var/lib/apt/lists/* \
    && locale-gen en_US.UTF-8

ENV LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8

# uv — managed Python + dependency installer (used by script apps)
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
ENV PATH="/root/.local/bin:${PATH}"
RUN uv python install 3.12

# Deno — TypeScript runtime (used by script apps and the Worker layer)
RUN curl -fsSL https://deno.land/install.sh | DENO_INSTALL=/usr/local sh

# BuckyOS root + canonical directory skeleton (see §9 of the container spec).
ENV BUCKYOS_ROOT=/opt/buckyos
WORKDIR /opt/buckyos

RUN mkdir -p \
    /opt/buckyos/bin/opendan \
    /opt/buckyos/data \
    /opt/buckyos/logs \
    /opt/buckyos/storage \
    /opt/buckyos/tools/store \
    /opt/buckyos/tools/bin \
    /mnt/buckyos/pkg

# OpenDAN binaries shipped by the image (Agent dispatch target).
COPY opendan /opt/buckyos/bin/opendan/opendan
COPY agent_tool /opt/buckyos/bin/opendan/agent_tool
RUN chmod 755 /opt/buckyos/bin/opendan/opendan /opt/buckyos/bin/opendan/agent_tool

# Unified entrypoint: instance-volume bootstrap + file-level upstream sync +
# agent/script/custom dispatch (replaces the old overlayfs/fuse bootstrap
# and the separate script-service entrypoint).
COPY entrypoint.sh /opt/buckyos/entrypoint.sh
RUN chmod 755 /opt/buckyos/entrypoint.sh

ENV AIOS_TARGET_ARCH="${TARGET_ARCH}" \
    BUCKYOS_INSTANCE_VOLUME=/opt/buckyos/instance \
    BUCKYOS_PKG_SOURCE_DIR=/mnt/buckyos/pkg \
    BUCKYOS_PKG_DIR=/opt/buckyos/instance/pkg \
    DENO_DIR=/opt/buckyos/instance/deno-cache \
    UV_CACHE_DIR=/opt/buckyos/instance/uv-cache \
    NPM_CONFIG_CACHE=/opt/buckyos/instance/npm-cache \
    PIP_CACHE_DIR=/opt/buckyos/instance/pip-cache

EXPOSE 3180

ENTRYPOINT ["/opt/buckyos/entrypoint.sh"]
