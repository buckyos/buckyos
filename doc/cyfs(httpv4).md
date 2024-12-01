# CYFS(httpv4)协议设计

## 内容包含
1. 从面向地址的协议变成面向身份的协议
支持  http://xxx.dev.did 这种形式的域名，并能直接使用去中心的基础设施来解析域名（不依赖传统域名解析服务器）

2. 支持ContentBase协议
ContentBase协议的目的是支持可信的多源获取。即当某个Node需要一个特定的Object时，可以使用NDN的基础设施来实现更高性能和更可靠的获取
高性能、高可靠的的基本逻辑
1. 就近获取，zone chunk mgr的反向代理实现可以做到加速
2. 多源获取：解决404问题
3. 同时获取（可选），通过将一个大文件切成多个chunk(mix_hash的计算过程天然就可以支持这种大分块),可以支持一个文件同时从多个源获取

- ContentBase协议的url规范

https://ndn.$zoneid/$objid/index.html?ref=www.buckyos.org （获得某个obj）,该
https://ndn.$zoneid/$path?objid=xxx&ref=www.buckyos.org （通过路径获得obj,如果路径不指向objid，则访问失败）
https://$objid.ndn.$zoneid/index.html?ref=www.buckyos.org （获得某个obj）
cyfs://$objid/index.html?ref=www.buckyos.org （获得某个obj）
cyfs://o/$objid/index.html?ref=www.buckyos.org （获得某个obj),使用传统的o链接的目的

任意http GET协议，可以在response中增加cid field，用于说明返回结果的objid

objid在域名里的编码法
1. objid的base58编码表达,编码时包含$has
2. 使用 $hash_$hashtype的方式表达


不同的HTTP方法含义

GET方法：获得Object
往Remote Zone的ObjectMgr里写入一个Object暂时不支持，更合适的逻辑应该是通过应用让Remote Zone在一个合适的时间点去主动获取Object（但服务器很难向Client获得Object）
GET Object的Cache时间基本是无限大的（对象不可变）

//PUT方法：向目标zone写入Object,PUT方法根据URL的特点可以说明是否要关联到某个具体路径

DELETE方法：删除Object或R路径

PATHC方法：通过DIFF的方法创建Object的新版本


Object DEX相关协议包含
DEX的核心是，多方共同认可一个可信的状态变化
S1 --Objec1 --> S2 --Object2 --> S3 .... 

 - ZoneA向ZoneB Push Object / Object List
 - ZoneA向特定MQTT broker订阅主题
 - ZoneB向特定的MQTT brokerf发布ObjectId




## Named Data 和 Named Object 的区别
objtype:objid

objtype 为hashtype时,objid为chunkid



## 通过ObjId构建Web3语义网的逻辑




## 概述
HTTP ---> 兼容的同时，定义了NDN/NON的传输协议的HTTP兼容表达，支持可信URL(O  Link) 
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



### GET Chunk 

HTTP Method:GET

Host mode:
http://$chunkid.[ndn.$zoneid]/index.html?ref=www.buckyos.org
我们在表达chunkid时，要注意其在host中的合法性。

Path mode(不推荐):
http://[$zoneid/ndn/]$chunkid/index.html?ref=www.buckyos.org

括号内的信息是可配置的，上述URL等价于
cyfs://$chunkid/index.html?ref=www.qq.com

通过chunkid定位到一个确定的文件后，http range也是有效的


#### PUT Chunk
PUT Chunk是幂等操作,语义与HTTP PUT语义一直。讲Chunk写入到一个确定的资源位置
PUT Chunk时，可以让chunk与某个逻辑路径绑定
PUT Chunk是非多源的，如果服务器上已经拥有了该Chunk，那么可以在客户端上传数据完成前提前返回成功

HTTP Method:PUT 

Host mode:
http://$chunkid.[ndn.$zoneid]/

## 传输证明

