FROM quay.io/coreos/etcd:v3.5.13 as etcd
FROM ubuntu:24.04
COPY --from=etcd /usr/local/bin/etcd /usr/local/bin/
COPY --from=etcd /usr/local/bin/etcdctl /usr/local/bin/
COPY --from=etcd /usr/local/bin/etcdutl /usr/local/bin/

EXPOSE 2379 2380

ADD target/release/node_daemon /usr/local/bin/

WORKDIR /buckyos

CMD ["node_daemon"]
