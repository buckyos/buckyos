# Gateway模式思考

## 客户端使用方式：连接NAT后面的node

本质上都是需要 `tcp.open("node_id",app_port)`

从实现上，如果客户端是我们自己开发的，或则已有客户端软件支持socks代理，那么就可以通过将gateway当作一个socks代理服务器，用标准的代理协议来连接本来连接不到的node。这一部分按现在Clash的架构，可以称作gateway-core, 其核心逻辑在于如何能连接上一个node,也是系统里主要逻辑的部分。

如果已有客户端不支持代理，就要通过透明代理技术来强制其使用socks代理 （这个代理不一定是gateway-core）。透明代理技术我们之前也研究了，无非就是下面几种方法

1. 通过域名解析(host文件或指向我们自己的nameservice)将无法连接的node_id转换到127.0.0.1,然后gateway-front刚好也监听了app_port,那么gateway-front会根据一个配置文件，去用socks代理协议请求gateway-core，目标是("node_id",app_port).这个技术的缺点是app_port冲突
2. Fake_ip, 通过域名解析将node_id解析成一个特定的，和node_id 一一对应的fake-ip,然后在通过iptables规则将发往该fake-ip的tcp连接都转发到一个特定的socks5代理服务器上
3. Docker， 通过容器启动指定进程，并将进程的全部流量转发到一个socks5代理服务器上 (需要研究，如果可行这个方案对我们来说最简单)

使用方案2的gateway-front和gateway-core配合，正是现在主流翻墙软件的架构(OpenClash),我想其体系里的front部分是可以考虑拿过来用的(如果独立性强的化）。
上述架构中，判断哪些流量走哪些路径就有了在front分流还是在core里分流的逻辑。front的分流是将直连分出去，core里分流还要区分用什么tunnel.

## 服务器使用方法：在NAT后面的node上的server，使用tcp.listen / accpet

虽然从技术上说，如果客户端的gateway-core足够强力，那么NAT后面的server只需要正常监听就能工作。但遗憾的是目前并没有这样的技术。我们需要在NAT后面的网络里，至少有一个gateway-rproxy (反向代理），实现如下路径

```
gateway-core ---tunnel--> gateway-rproxy -----> server@node
```

tunnel协议的核心，就是gateway-core告诉gateway-rproxy,要建立一条到 server@node的连接，于是gateway-rproxy：

0. gateway-core用tunnel协议告诉gateway-rproxy要建立到server@node的连接
1. gateway-rproxy建立到server@node的tcp连接A，
2. gateway-rproxy建立到gateway-core的连接B，并绑定到A
3. gateway-core把连接B与一个socks5 client session绑定。

从而实现了 client---->server@node的连接。
按上述逻辑，gateway-core,gateway-rproxy的存在是可以按网络环境来的，一个网络环境有一个实例就够了。