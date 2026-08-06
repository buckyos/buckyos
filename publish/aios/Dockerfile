FROM ubuntu:24.04

ARG TARGET_ARCH

LABEL org.opencontainers.image.title="paios/aios (BuckyOS Worker Image)" \
      org.opencontainers.image.description="Unified worker runtime for BuckyOS apps and OpenDAN agents" \
      org.opencontainers.image.vendor="BuckyOS DAO"

ENV DEBIAN_FRONTEND=noninteractive

# Runtime + predefined worker CLI tools. No compile toolchain: the Crafter
# image (deferred per requirements §7.8) is the only place build-time deps
# are allowed to land. Heavy tools (FreeCAD etc.) live in the paios/exttool
# image and arrive via the buckyos-exttool named volume, not this image.
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

# BuckyOS Agent AICC CLI tools. Keep these outside /opt/buckyos/tools because
# that path is shadowed by the shared ExtTool volume at runtime.
COPY buckyos-agent /opt/buckyos/bin/opendan/buckyos-agent
COPY aicc-tool-launcher /usr/local/bin/aicc-tool
RUN chmod 755 /usr/local/bin/aicc-tool && \
    for command_path in /opt/buckyos/bin/opendan/buckyos-agent/commands/*.ts; do \
      command_name="$(basename "$command_path" .ts)"; \
      ln -sf aicc-tool "/usr/local/bin/$command_name"; \
    done && \
    deno check \
      --config /opt/buckyos/bin/opendan/buckyos-agent/deno.json \
      --import-map /opt/buckyos/bin/opendan/buckyos-agent/installed-import-map.json \
      /opt/buckyos/bin/opendan/buckyos-agent/aicc-tool.ts && \
    command -v aicc-tool >/dev/null && \
    command -v gen_image >/dev/null && \
    command -v edit_image >/dev/null && \
    aicc-tool --help >/dev/null 2>&1 && \
    gen_image --help >/dev/null 2>&1 && \
    edit_image --help >/dev/null 2>&1

# ExtTool Volume mount point — kept empty in the Worker image. Real payload
# ships in the separate paios/exttool image and lands in the buckyos-exttool
# named volume, which node_daemon mounts here read-only at run time.
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
    BUCKYOS_EXTTOOL_DIR=/opt/buckyos/tools \
    DENO_DIR=/opt/buckyos/instance/deno-cache \
    UV_CACHE_DIR=/opt/buckyos/instance/uv-cache \
    NPM_CONFIG_CACHE=/opt/buckyos/instance/npm-cache \
    PIP_CACHE_DIR=/opt/buckyos/instance/pip-cache \
    PATH="/opt/buckyos/tools/bin:${PATH}"

EXPOSE 3180

ENTRYPOINT ["/opt/buckyos/entrypoint.sh"]
