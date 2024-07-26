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
    sed -i '/option base-port/ c\    option base-port 49152' /etc/glusterfs/glusterd.vol; \
    sed -i '/option max-port/ c\    option max-port 49162' /etc/glusterfs/glusterd.vol; \
    sudo apt-get install -y samba; \
    sudo apt-get install -y fuse; \
    sudo apt-get install -y nginx; \
    apt clean;

EXPOSE 8080 2379 2380 139 445 49152-49162 3452

ADD target/release/node_daemon /usr/local/bin/
COPY smb_service/ "/buckyos/service/smb_service#0.1.0/"
COPY frame-glusterfs/ "/buckyos/service/glusterfs#0.1.0/"
COPY backup_service/script/ "/buckyos/service/backup_service#0.1.0/"
COPY target/release/backup_service "/buckyos/service/backup_service#0.1.0/"
COPY nginx_service "/buckyos/service/nginx_service#0.1.0/"

WORKDIR /buckyos

CMD ["node_daemon"]
