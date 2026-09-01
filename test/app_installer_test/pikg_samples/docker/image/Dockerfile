ARG BASE_IMAGE=busybox:1.36.1
FROM ${BASE_IMAGE}

COPY entrypoint.sh /entrypoint.sh

RUN chmod +x /entrypoint.sh

EXPOSE 80

ENTRYPOINT ["/entrypoint.sh"]
