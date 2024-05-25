FROM quay.io/coreos/etcd:v3.5.13 as etcd
FROM ubuntu:24.04
COPY --from=etcd /usr/local/bin/etcd /usr/local/bin/
COPY --from=etcd /usr/local/bin/etcdctl /usr/local/bin/
COPY --from=etcd /usr/local/bin/etcdutl /usr/local/bin/

RUN apt update; \
    apt install sudo -y; \
    DEBIAN_FRONTEND=noninteractive apt-get install -y tzdata python3 wget; \
    wget -O /usr/bin/systemctl https://raw.githubusercontent.com/gdraheim/docker-systemctl-replacement/master/files/docker/systemctl3.py; \
    chmod +x /usr/bin/systemctl; \
    sudo apt-get install -y glusterfs-server; \
    sudo apt-get install -y samba; \
    apt clean;

EXPOSE 2379 2380 139

ADD target/release/node_daemon /usr/local/bin/
COPY smb_service/ "/buckyos/service/smb_service#0.1.0/"
COPY frame-glusterfs/ "/buckyos/service/glusterfs#0.1.0/"

WORKDIR /buckyos

CMD ["node_daemon"]
