# zone的集群化

任何Zone，要么其ZoneBootConfig有配置SN，要么ZoneBootConfig有配置GatewayNode,否则公网访问就会有受限

## 默认Zone的设备都在同内网

查找OOD：
    使用多种局域网广播方法查找并与OOD建立链接

查找设备
    链接上OOD后，通过SystemConfig上的device info,来链接特定的device

Zone外设备连接Zone
    连接Zone的SN（在公网），通过SN转发到达内网：笔记本->SN->Zone内设备
    Zone内设备漫游到了外网时，可以通过tunnel协议更高效的连接Zone内的任意服务。如果SN只允许OOD1 keep-tunnel,那么这里就要做转发：笔记本->SN->OOD1->Zone内设备

## Zone有ServerNode在公网
该ServerNode会成为GatewayNode ,有IP或hostname
ZoneBootConfig的oods段，可看到GatewayNode的信息
Zone内的设备会与ZoneGatewayNode Keep Tunnel

查找OOD/查找设备
    与GatewayNode通信，了解设备信息

连接设备
    通内网直接连接，否则通过GatewayNode中转


## 按照集群初始化

当有条件的时候，应基于终端产品，尽量在初次建立Zone的时候就初始化为集群

从单OOD演进：
    - 从无GatewayNode到有GatewayNode时，必然进行一次集群 进化（需要修改ZoneBootConfig)
    - 当ServerNode>=3个时，提示用户进行一次进化。从单OOD变成3 OOD系统