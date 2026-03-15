# Boot Gateway配置的生成逻辑

> BootGateway的配置构造时没有调度器，所以不依赖调度器构造的配置均放在这里
> 进一步简化？3-OOD系统已经是中型系统了，是否可以要求必然可以直连（不会出现中转的情况？）
  产品上：公司一台，家里一台，父母家一台 这种场景组3-OOD可能是普通人实现私域安全的顶配了
> 大型系统：OOD会跨LAN，但是一定有专线了
> 永远是直连优先，优先级:ipv4>ipv6>rtcp转发

## OOD节点
BOOT阶段：OOD之间互相建立建立（如果有2n+1个OOD,至少要和n个OOD建立链接）
OOD 互相之间会尽力keep rtcp tunnel,并减少对SN的依赖
- OOD之间保持直接的rtcp-tunnel
- OOD之间通过SN保持rtcp-tunnel
- OOD之间，通过Zone-Gatweay,保持rtcp-tunnel。 
基于上面结构构造node_route_map

OOD会基于zone-boot-config,和其它OOD、SN/Zone-Gateway keep tunnel
- 通过局域网搜索找到其它OOD

> 由于rtcp stack只在cyfs-gateway,所以其它进程访问system_config是两个
- http://127.0.0.1:3200/kapi/system_config (最短路径)
- http://127.0.0.1:3180/kapi/system_config


## 非OOD的ZoneGateway节点

BOOT阶段:
- 被标注为ZoneGateway的节点等待成为OOD keep-tunnel的target
- 一旦有OOD连上，则主动与OOD通讯，连接上system_config

> 产品上，通常是用户自己架设的公网VPS,以脱离对SN的依赖

## 非OOD节点
BOOT阶段目标: 连接上system_config

1. 通过ZoneGateway肯定能连接上
2. 如果家用环境，ZoneGateway的域名解析指向SN，会导致SN失效后无法链接SystemConfig
- node_daemon有局域网设备发现逻辑，可以在局域网扫描找到OOD，进而建立boot阶段的NODE_ROUTE_MAP
- 短路径 Node--rtcp-->OOD
3. 在ZoneGateway不可用的情况下，可以通过阅读zone-config,尝试通过SN连接
- 短路径 Node--rtcp-->SN--rtcp-->OOD

4. 一旦能从SysetemConfig获得信息，就能建立更高效的NODE_ROUTE_MAP
- 旧路径 Node--https->SN--https->ZoneGateway--rtcp-->OOD
- 短路路径 Node--rtcp-->OOD
- 短路路径 Node--rtcp-->SN--rtcp-->OOD
- 短路路径 Node--rtcp-->zone-gateway--rtcp-->OOD

> 由于rtcp stack只在cyfs-gateway,所以非OOD的其它进程访问system_config就是两个URL
- https://zoneid/kapi/system_config
- http://127.0.0.1:3180/kapi/system_config

为什么必须建立到OOD/其它Node的rtcp路径？buckyos把对等看做常态
当client--tunnel--> server后 ，server就可以在需要的时候向client发起请求。减少了全局的轮询数量，同时也为ndn的大文件夹上传场景，提供了“全client"的思路：有逻辑上的server执行client逻辑，通过向client主动发起get请求来实现“上传"


## 结论1：BootGateway中的内容

BuckyOS cyfs-gateway的基本运行结构定义

### NodeGateway （OOD,Node,Client）
- rtcp-stack
  - 对Zone内中转提供支持
- node-tcp-stack@3180 -> node-gateway-http-server
  - 只允许127.0.0.1访问（请求都来自rtcp-stack转发）
- node-gateway-http-server:处理HTTP请求并分发
  -> 本机服务进程
  -> 通过rtcp协议到达其他node-gateway

NodeGateway的配置是主逻辑

### ZoneGateway
目前无特别构造

### Light Client's Gateway (无system_config)
 - rtcp-stack
 - node-tcp-stack@3180 -> node-gateway-http-server
 - node-gateway-http-server:处理http请求，并发送到zone-gateway
   -> 通过rtcp协议到达zone-gateway -> 分到具体service_provicer

Light Client's Gateway相比直接用https有2个优势
- 逻辑上不依赖https协议以及CA
- 自带身份，可以跳过一些面向web的OAuth流程

## 结论2: CYFS-Gateway的配置构造OverView
1. Boot阶段会构造Node_Route_Map,并设置keeptunnel-list
2. Boot阶段会对努力对局域网的OOD进行扫描，并会对扫描结果进行缓存
3. 调度器会根据SytemConfig,构造
   - 新的NodeRouteMap
   - AppInfo
   - ServiceInfo
   - Zone Gateway
    - tcp_stack@80 -> node-gateway-http-server （有安全风险，可以默认不开）
    - tls_stack@443,并持有正确的zone tls证书 ->node-gateway-http-server
    - acme挑战，自动维护tls的证书