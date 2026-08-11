## DNS-resolver的问题

DNS direct boot 查询 不是被包含在resolve_did 里了么？
> 其实就是name-client的cache机制还不够完整，无法插入 “did:web:xxx 的 owner 是 did:bns:xxx" 这样的信息？

DNS rsolver的Cache update:要多插入gateway_device的doc OK 



## 核心环境变量的意义整理 OK 

环境变量都是已经验证过的，关键Doc，方便在脚本中直接使用

BUCKYOS_ZONE_DOC 里面放 ZoneDocument JSON；调度器构造的是 ZoneConfig，不是 ZoneBootConfig
BUCKYOS_ZONE_CONFIG 保留
BUCKY_ZONE_OWNER （删除)
BUCKYOS_THIS_DEVICE 保留, BUCKYOS_THIS_DEVICE_INFO(删除)


## booting状态下(resolve-did)行为特例说明

- 通过node-finder,在did-cache中得到target device document (注意需要DV Test中验证）

- 梳理SessionToken的集中构造路径鱼RootTrust的关系

- resolve_did("did:web:ood2.xxx.com","device")

在权威源没有数据的情况下，如何判断 did->owner: a.推断 b.共主 ?
因此 did:web:xxx.com ，和 resolve_did("did:web:xxx.com","zone").owner 都是合法的Owner

因为还没启动，所以无法通过标准的web-resolver得到device_doc
1）Zone内通过finder可能已经发现了，通过cache短路得到
2）如果ood2曾经主动连接过当前设备,后续通过cache短路得到
3）通过sn-resolver可以查到





## Zone内 OOD/Node等设备的doc初始化问题

1）固定在first_ood (ood leader) 上完成scheduler boot
2）OOD节点在连接上system_config后，立刻注册自己的device_doc+update device_info


scheduler boot 初始化不写死 ood1，但使用 zone_document.oods.first().unwrap()，只初始化第一个 OOD 的 device doc/node config。
> 这可能是为了root trust? 为了首次有资源可以用（那这个在多ood下就是一个bug) : 单ood立刻分配，多OOD则需要多等一会（等所有的ood的doc都上报了）

system_config的device_doc是如何构造的？
> OOD设备自注册 or 管理员手工注册？ 先走自注册 + 定期update device_info的流程

按现在的流程，zone node 之间建立连接时，
1）client主要是要得到正确的ip (device_info) 并得到target device的可信公钥（验证过的device_document)
2) server端(cyfs-gateway内部）：获取from_device_doc不重要，重要的是获取owner的签名后能对from_device_doc_jwt进行验证



## 实现BuckyOS Zone-Resolver

核心功能 BuckyOS 的 Zone，它主要是解决自己 Zone 内的 DID 解析. 并会使用本地Cache
- 稳定的OwenrDocument验证（会合并真实源上的OwnerDocument)



对Zone-Resolver 在 DV 环境下进行完整的测试

name_client初始化的时候,zone-resolver默认开启，但是cyfs-gateway可以显示配置关闭 
> zone-resolver内部实现时，要注意用关闭zone-resolver的name-client接口，否则会循环依赖


## 实现SN Zone-resolver

### 通过内部逻辑获得 did:web:xxx的owner

支持获取did:web:xxx 的owner
支持获取did:web:ood1.xxx 的owner








