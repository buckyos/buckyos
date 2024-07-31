# 在外（出差）时访问系统

## BuckyOS的Internet Access 的目标

核心目标：不管用户在哪里，只要能访问互联网，就能以一致的方法使用BuckyOS

实现上述目标的难易度排名：

最简单：用户基于App使用BuckyOS, 此时用户看不到任何的URL，只要App能处理好使用什么协议，用户的感觉就是透明的

中等：用户使用VPN技术(包括SBOX)后，让用户的客户端设备漫游回家里的网络，这种方法也有非常好的兼容性，但需要用户要么会设置VPN(还好现在主流的系统里都支持VPN了），要么要有一个SBOX。使用SBOX我们的自由度更大，是我们在这个方向上首推的方法。

最难：用户在任意机器上使用浏览器访问BuckyOS，这意味着我们必须在标准协议的框架内实现BuckyOS的访问，而且还需要设计一个“永久性”的URL策略。使用浏览器+URL的方式是我们的最终目标，我们的技术改进路线是从浏览器插件到我们自己的cyfs浏览器。


## 一、Internet Access的局限性

Internet Access的本质困难在于Zone的绝大多数设备（尤其是OOD）运行在NAT后面，这意味着Zone内的设备是无法直接被Internet访问的。从技术上说，到OOD的通信信道无法就是下面3种：

0. （一定成功）OOD本身拥有公网地址（IPv6支持），但这种情况太少了，用户为了安全也会要求设备在NAT后面
1. （不一定成功）通过P2P穿透实现直接通信，P2P的握手过程通常需要一个公网节点(SN)的协助
2. （不一定成功）通过配置端口映射实现直接通信，这种方法需要用户的网络设备支持UPnP或者用户能手动配置端口映射
3. （一定成功）通过一个公网节点转发流量。


基于上述方法，我们尽量让用户能基于如下的 https://$servicename.$zoneid/page1.html url 来访问BuckyOS.

## 启用Internet Access

从原理上，并不是所有的Zone都可以从互联网(Zone外)访问，但随时随地的访问BuckyOS和运行在BuckyOS上的dApp又是一个移动互联网时代非常刚性的需求。因此，我们需要引导用户配置以支持启用BuckyOS的Internet Access功能。

### 通过Enable Gateway Node来启用Internet Access

这也是我们最推荐的方法：通过激活一个拥有固定IPv4地址的VPS节点，可以为BuckyOS提供稳定的Internet Access能力。用户有2种方法来获得这个节点：

1. 通过内置的Gateway Service订阅购买
2. 通过一个URL/二维码添加一个待激活的Gateway Node来实现，我们提供简单的安装流程，可以让中高级用户把自己的VPS节点配置成Gateway Node。


### 通过端口映射来启用Internet Access

系统检测是否生效，并给出打开端口映射的方法。我们只需要开放80/443端口就足够了。
无法通过操作Control Panel实现打开端口映射

### 通过P2P技术来启用Internet Access （拥有Gateway Node有时可以提高P2P的连通率）

系统检测NAT类型，并给出P2P目前的支持情况。
一般来说，需要配置一个BDT-SN节点和BDT Reply节点来确保P2P的100%可用。所以这里有一个配置来提高成功率。


### 混合上述集中方法

有一些Gateway的服务商可能有流量/带宽的限制，因此在上述两种方式都支持的情况下，有条件应该优先使用端口映射的方式启用Internet Access。



## 二、Internet Acess 配置面板

显示是否启用
如有Gatewway Node,显示其状态

显示目前Zone的Internet Acess能力和OOD的网路情况

- 是否Enable GatewayNode
- 是否启用DDNS（无GatewayNode情况下）
- 探测的OOD网络情况：是否在NAT后面，NAT类型，是否启用端口映射,IP地址信息，接入状态信息等

## 三、使用SBOX远程访问系统

出差时有互联网：

通过连接SBOX提供的Wifi,相当于接入了OOD所在的局域网，可以像在家里一样使用所有的功能

出差时无互联网：

通过连接SBOX提供的WiFi,并依赖SBOX上的存储缓存，DFS是只读的，可以受限的读取DFS系统里的热数据。


### 上述需求对SBOX的要求

- 相当于在离线环境下单机，脑裂模式运行BuckyOS,如果该BuckyOS里只有一个SBOX，那就是全功能的
- 当支持了基于用户数据的，用户能干预的数据合并，那么SBOX就可以运行某些特定dApp在离线状态下写入
- 基于cyfs-gatway的"VPN"
- 能随时运行dApp(保存了所有dApp的应用镜像)
- 能在离线状态下，通过缓存得到DFS里最近访问的数据。因此SBOX上有DFS的MetaData，有CacheData,但是基本没有ChunkData



## 四、Zone间通信的基本框架

Email模式,通过Post Named-Object实现功能，写请求是可缓存的，幂等的
主要依靠cyfs-gateway
主要行为是 Read Named-Object

