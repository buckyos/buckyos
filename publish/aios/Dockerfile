FROM ubuntu:24.04

ARG TARGET_ARCH

LABEL org.opencontainers.image.title="AIOS" \
      org.opencontainers.image.description="OpenDAN runtime image for paios/aios" \
      org.opencontainers.image.vendor="BuckyOS DAO"

WORKDIR /opt/buckyos

RUN mkdir -p /opt/buckyos/bin/opendan

COPY opendan /opt/buckyos/bin/opendan/opendan

ENV AIOS_TARGET_ARCH="${TARGET_ARCH}"

EXPOSE 3180

ENTRYPOINT ["/opt/buckyos/bin/opendan/opendan"]
