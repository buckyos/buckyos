# Project Posal
cyfs gateway是下一代用户态的网络协议栈，是buckyos第一个版本中难度最大的组件之一。
首个版本的目标是
1. 明确功能边界和API实现方式
2. 支持etcd集群的垮NAT启动




## 主要功能
0. 不包含名字解析，名字解析由cyfs name service提供.剥离出来后适用性更强
1. 基于deviceid的可信，可穿透tunnel的建立
2. 在有tunnel的基础上，协助进行标准的tcp/udp通信：化不通为通
3. 为pkg mgr 提供content based的可信数据下载能力 (NDN)
4. 连接bakcup server, 进行数据备份/恢复

## 接口设计
cyfs gateway的接口不应该有任何L2的东西，并不能假设必须和某个iptable规则混合使用
1. Socket5 代理(注意对身份的管理)
2. 反向代理注册
3. NDN的标准HTTP协议支持



## 关键难点

处于两个不同NAT后面的cyfs gateway如何建立tunnel?
```
#app-client
virtual_ip = cyfs_lookup(deviceid).get_virtual_port()
tcpstream = tcp_connect(virtual_ip, port)

#cyfs gatewaye 1
def socket5_on_tcp(clients,serverip,serverport):
    deviceid = cyfs_lookup(serverip)
    if deviceid:
        tunnel = get_bdt_tunnel(deviceid)
        stream,tunnel.create_stream(port)
        bind(clients,stream)

# cyfs gateway 2
def on_stream(newstream,server_port):
    register_server = get_register_server(server_port)
    tcp_stream = tcp_connect(register_server.ip,register_server.port)
    bind(tcp_stream,newstream)

#app-server
def on_tcp_stream(newstream):
    req = newstream.read()
    resp = process(req)
    newstream.write(resp)

```

针对传统应用，通过Virtual IP的方式来实现功能