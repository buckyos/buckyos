FROM ubuntu:24.04

ARG TARGET_ARCH

LABEL org.opencontainers.image.title="AIOS" \
      org.opencontainers.image.description="OpenDAN runtime image for paios/aios" \
      org.opencontainers.image.vendor="BuckyOS DAO"

WORKDIR /opt/buckyos

# Required by OpenDAN runtime and common agent workspace workflows.
RUN apt-get update && apt-get install -y --no-install-recommends \
    tmux \
    bash \
    ca-certificates \
    fuse-overlayfs \
    python3 \
    python3-pip \
    python3-venv \
    nodejs \
    npm \
    git \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /opt/buckyos/bin/opendan

COPY opendan /opt/buckyos/bin/opendan/opendan
COPY agent_tool /opt/buckyos/bin/opendan/agent_tool
RUN chmod 755 /opt/buckyos/bin/opendan/opendan /opt/buckyos/bin/opendan/agent_tool

ENV AIOS_TARGET_ARCH="${TARGET_ARCH}"

EXPOSE 3180

ENTRYPOINT ["/opt/buckyos/bin/opendan/opendan"]
