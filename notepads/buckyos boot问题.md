## DNS-resolver的问题

DNS direct boot 查询 不是被包含在resolve_did 里了么？ 
> 其实就是name-client的cache机制还不够完整，无法插入 “did:web:xxx 的 owner 是 did:bns:xxx" 这样的信息？

DNS rsolver的Cache update:要多插入gateway_device的doc OK 
是否要约定，有boot 等于 有zone-document? 显示大于隐式！


## 3个环境变量的意义？

BUCKYOS_ZONE_BOOT_CONFIG 里面放真的ZoneConfig（包含了Zone-Document)? 不过感觉这个环境变量似乎从来没有用过
    调度器构造的是ZoneConfig，不是ZoneBootConfig

BUCKY_ZONE_OWNER用么？（稳定Owner？）=> 应该改成设置OwnerDocument , 在name_client中添加到权威cache中 （是并集的关系，OwnerDocument在BNS 上更新后，新的签名也是有效的？（Zone owner是一个合成出来的？）
BUCKYOS_THIS_DEVICE , BUCKYOS_THIS_DEVICE_INFO 的使用


## booting状态下(resolve-did)行为特例说明

- 通过node-finder,在did-cache中得到target device document (注意需要DV Test中包换）

梳理SessionToken的集中构造路径鱼RootTrust的关系

did:web:xxx 的自Owner流程分析

## Zone内 OOD/Node等设备的doc初始化问题

scheduler boot 初始化不写死 ood1，但使用 zone_document.oods.first().unwrap()，只初始化第一个 OOD 的 device doc/node config。
> 这可能是为了root trust? 感觉挺奇怪的

system_config的device_doc是如何构造的？
> OOD设备自注册 or 管理员手工注册？ 先走自注册 + 定期update device_info的流程

按现在的流程，zone node 之间建立连接时，
1）client主要是要得到正确的ip (device_info) 并得到target device的可信公钥（验证过的device_document)
2) server端(cyfs-gateway内部）：获取from_device_doc不重要，重要的是获取owner的签名后能对from_device进行验证



## 实现BuckyOS Zone-Resolver

对Zone-Resolver 在 DV 环境下进行完整的测试

name_client初始化的时候,zone-resolver默认开启，但是cyfs-gateway可以显示配置关闭 
> zone-resolver内部实现时，要注意用关闭zone-resolver的name-client接口，否则会循环依赖


## 实现SN Zone-resolver

### 通过内部逻辑获得 did:web:xxx的owner

支持获取did:web:xxx 的owner
支持获取did:web:ood1.xxx 的owner










