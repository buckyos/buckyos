# SDN(网络管理)



对家庭用户来说，网络管理功能的需求相对简单，基本集中在Parental Control（家长控制）和隐私管理上(Private Relay)。而对企业用户来说，网络管理功能则更加复杂，需要支持VLAN、QoS、VPN等功能。 在BuckyOS的早期版本里，我们更多的是把底层架构做好（哪些是系统的功能，哪些是可以持续扩展的），在这块的UI需求上我们限只关注家庭需求。



## cyfs-dns 独立产品化

对w3c的did标准完整支持，同时也有支持我们的dns->did转换查询


## cyfs-gateway 独立产品化

（依赖cyfs-dns）

Front：拦截流量到内核（透明代理）
Proxy-Core：可以根据条件配置(规则引擎），管理到目标主机的tunnel.通过扩展可以支持更多的tunnel
bdt-stack.client:发起p2p连接。
bdt-stack.forword:处理p2p协议的转发，可以透明的给旧服务添加P2P支持，一个热门的forwrd就是forward到nginx
cyfs-server：识别cyfs(httpv4)的特殊请求，并进行处理,一般是nginx的upstream



因为网络是双向的，所以通常cyfs-gateway里通常同时包含Proxy-Core和cyfs-server,bdt-forword



