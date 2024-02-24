# CYFS(httpv4)协议设计

## 动机


## 概述
HTTP ---> 兼容的同时，定义了NDN/NON的传输协议的HTTP兼容表达，支持可信URL(R Link) 
TCP ---> Stream， 扩展了NDN传输/NON传输
DNS ---> Decentralized Name Service
IP  ---> Tunnel，通过公钥进行寻址

``` 下面2个是等价的
http://2DFD1FCFC9601E7De871b0BbcBCbB6Cad6901697.cyfs.com/index.html
cyfs://2DFD1FCFC9601E7De871b0BbcBCbB6Cad6901697/index.html (既可以是标准http，也可以是rlink ，看http的返回头)
cyfs://o/2DFD1FCFC9601E7De871b0BbcBCbB6Cad6901697/$hash_of_index.html （我们最重要的o link）
cyfs://o/2DFD1FCFC9601E7De871b0BbcBCbB6Cad6901697/$dir_objid/index.html_hash
```
## CYFS的兼容性设计
和之前的实现相比，新版的实现我们应该强化兼容设计。让不同的场合下，只使用部分也都是可以的。
按使用的难度从低到高，我们分为下面几个层次
1. 使用CYFS的dns,可以可靠的把一个公钥转成一个ip或一个域名 （返回值和DNS相同）。这可以比较简单的让用户获得一个永久的，不需要续费的域名
2. 使用CYFS的tunnel组件。基于TCP/IP开发（包括HTTP）的 client/server无需任何修改，只需要各自都有CYFS gateway就可以连接上（经典的remote desktop的例子）
client  ---> cyfs gateway ---> server
CYFS gateway最好是运行在公网的服务，这样可以确保系统的可靠连通性（没有NAT的问题）
`在这一层,就可以实现所有传统服务在NAT后面的可用，也应该是我们重点推荐的方式`

bdt提供的开发组件里，也鼓励开发者用非常低的学习和配置成本，可以只使用tunnel层。
3. 完整的使用CYFS提供的新功能，包括object link ,Rootstate link, 并启用NDN传输等


## 去中心的域名解析
一个DNS服务，可以用标准的DNS协议解析 公钥->IP(域名)
用户可以在BTC、ETH网络（持续增加）上为自己的公钥配置域名



## Tunnel协议 （P2P）
只要Tunnel是通的，就可以确定性的建立P2P连接
Tunnel 有一点保活的成本


## 兼容HTTP的Object Link 

## NON传输

## NDN传输


## 传输证明

