FROM ubuntu:24.04

ARG TARGET_ARCH

LABEL org.opencontainers.image.title="BuckyOS Script Service" \
      org.opencontainers.image.description="Runtime image for BuckyOS HostScript applications (Python/uv + TypeScript/Deno)" \
      org.opencontainers.image.vendor="BuckyOS DAO"

RUN apt-get update && apt-get install -y --no-install-recommends \
    bash \
    ca-certificates \
    curl \
    unzip \
    && rm -rf /var/lib/apt/lists/*

# Install uv (Python toolchain manager)
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
ENV PATH="/root/.local/bin:$PATH"

# Bootstrap a managed Python so scripts work out of the box.
RUN uv python install 3.12

# Install Deno
RUN curl -fsSL https://deno.land/install.sh | DENO_INSTALL=/usr/local sh

RUN mkdir -p /opt/script/package /opt/script/data

COPY entrypoint.sh /opt/script/entrypoint.sh
RUN chmod +x /opt/script/entrypoint.sh

ENV SCRIPT_PACKAGE_ROOT="/opt/script/package"
ENV SCRIPT_DATA_ROOT="/opt/script/data"
ENV AIOS_TARGET_ARCH="${TARGET_ARCH}"

WORKDIR /opt/script

ENTRYPOINT ["/opt/script/entrypoint.sh"]
