##### zone节点启动流程

1. 读取本地配置，判断本地配置中是否存在etcd配置，如果存在则直接进入步骤4
2. 启动nameservice服务
3. 调用nameservice接口获取zone配置
4. 根据配置判断在本地是否启动etcd，如果需要启动则启动etcd
5. 为nameservice配置etcd并重启
6. 启动其它进程

##### 本地配置

节点激活程序可以生成本地配置，格式如下：

```json
{
	"name": "node1.example.zone",
    "ca": "根证书",
    "cert": "节点证书",
    "key": "节点私钥",
    "etcd": [{
            "node_name": "node1.example.zone",
            "protocol": "https|cyfs",
            "address": "",
            "peer_port": 2380,
            "client_port": 2379
        },{
            "node_name": "node2.example.zone",
            "protocol": "https|cyfs",
            "address": "",
            "peer_port": 2380,
            "client_port": 2379
        },{
            "node_name": "node3.example.zone",
            "protocol": "https|cyfs",
            "address": "",
            "peer_port": 2380,
            "client_port": 2379
        }]
}
```

激活时必须设置配置包括：name、ca、cert、key

etcd配置可以配置也可以不配置，如果配置了节点启动时将直接从本地读取，如果没有配置将从nameservice中获取，如果nameservice获取失败，则启动失败。

##### zone nameservice配置

```json
{
    "name": "example.zone",
    "type": "zone",
    "addr_info": [{
        "protocol": "https",
        "address": "192.168.1.1",
        "port": 3456
    }],
    "api_version": "v1",
    "extend": {
        "etcd":[{
            "node_name": "node1.example.zone",
            "protocol": "https|cyfs",
            "address": "",
            "peer_port": 2380,
            "client_port": 2379
        },{
            "node_name": "node2.example.zone",
            "protocol": "https|cyfs",
            "address": "",
            "peer_port": 2380,
            "client_port": 2379
        },{
            "node_name": "node3.example.zone",
            "protocol": "https|cyfs",
            "address": "",
            "peer_port": 2380,
            "client_port": 2379
        }]
    },
    "sign": "根证书签名"
}
```

##### etcd启动配置

1. 如果etcd之间需使用tls加密，并且需要验证身份。

[详情请见]: https://etcd.io/docs/v3.5/op-guide/security/	"Transport security model"

etcd启动配置

```sh
$ etcd --name node1 --data-dir infra0 \
  --client-cert-auth --trusted-ca-file=/path/to/ca.crt \
  --cert-file=/path/to/node1.crt --key-file=/path/to/node1.key \
  --advertise-client-urls https://${node_ip}:2379 --listen-client-urls https://${node_ip}:2379 \
  --peer-client-cert-auth --peer-trusted-ca-file=/path/to/ca.crt \
  --initial-advertise-peer-urls=https://${node1_ip}:2380 --listen-peer-urls=https://${node_ip}:2380 \
  --peer-cert-file=/path/to/node1.crt --peer-key-file=/path/to/node1.key \
  --initial-cluster-token etcd-cluster \
  --initial-cluster infra0=https://${node1_ip}:2380,infra1=https://${node2_ip}:2380,infra2=https://${node3_ip}:2380 \
```

请求配置

```sh
$ curl --cacert /path/to/ca.crt --cert /path/to/client.crt --key /path/to/client.key \
  -L https://127.0.0.1:2379/v2/keys/foo -XPUT -d value=bar -v
```

