## DID和可信身份
DID的一般设计用途，是允许身份的拥有者，不依赖任何系统，不可篡改的对身份进行验证（类似联合登录，用户只需要创建一次账号，就可以在所有支持DID的系统里使用该账号）
通过DID可以可信的得到公钥 （AuthKeyList)
语义did (did_name) 需要通过至少一次网络通信才能得到AuthKeyList
而did_with_key则不需要网络通信就可以直接得到AuthKey
    - 语义连接的可信，关键在于解析did_document过程的可信
    - 非语义链接，本质上要在链接中嵌入公钥信息，比较适合指向不会修改的内容


我们在构造语义链接时，必须使用语义did
```
auth_key = did.to_auth_key()
if auth_key.is_some() {
    did.is_self_auth() 
} else {
    auth_key = resolve_auth_key(did,kid) //语义did
}
```

## DID<->DID-Document 与 ObjID<->NamedObject

通过DID可以得到DID Document,该对应关系的可信性由did.method保障
通过ObjID可以得到NamedObject,ObjId的构造本质上上其指向的NamedObject的Hash，因此无需网络就可以验证
用JWT包装的DID Document/NamedObject ，可以通过关联的OwnerDid,来确定该“文档确实是由某人创建的”，对该行为进行验证有需要对OwnerDid的Authkey进行可信验证
    对resolve_auth_key来说，时间点也是很重要的。基于区块链来保存DID Document，可以跟踪auth_key的修改记录，并得到文档创建时间时的正确的auth_key

DID和ObjId的区别:DID的核心是可以得到auth_key(resolve或to)，ObjId则用来指向一个不可变的命名数据
DIDDoc和NamedObject的区别：
    - DID指向的Doc可以改变，但只有最新的那个是合法的
    - ObjId指向的NamedObject不可以修改
    - DIDDoc有一些固定字段的要求，NamedObject没有
    - ??DIDDoc一定可以验证其创建者(便于缓存？），而NamedObject不一定能验证 ?？ 
    - 因为要构造JWT的原因，需要构造JWT的NamedObject的定义里都必须要有exp



```
did_doc_jwt = resolve_doc(语义DID)
auth_key = did_doc.get_auth_key(kid)
verify_did_doc(did_doc_jwt,auth_key)

function resolve_auth_key(did,kid) {
    did_doc_jwt = resolve_doc(did) 
    auth_key = did_doc.get_auth_key(0) //0号，或则叫默认AuthKey,应能对did_dow_jwt进行校验 
    verify_did_doc(did_doc_jwt,auth_key)
    return did_doc.get_auth_key(kid)
}
```

## ZoneID 与 DID
ZoneID是一种特殊的DID，其设计目标是能支持通过ZoneID建立与Zone的可信连接。因此ZoneConfig（ZoneId对应的DID Document）设计的主要用途是
- 返回一组可用于建立链接的公钥 
- 包含Zone Cluster的boot info,可以完成启动引导 : oods，
- 包含建立链接的信息 
  - oods中的ood string可能包含固定的地址信息 ood1#70.230.11.103
  - sn信息（用于方便非TCP链接）

解析Zone Did的优先级
    - 通过区块链解析
    - 通过https解析：比用DNS解析更可靠
        我都能通过https获取zone的did了，那我还要连接他干嘛？要用tunnel连接，因此先用https协议获得zone-config
    - 用过DNS解析：返回的JWT需要可信的知道Zone的Owner才可以校验，该流程一般只用于Zone内的boot流程

可以防御的攻击
- Zone内： 
  - 通用验证公开的Zone DID Document的签名，可以确保Zone Config没有被解析服务提供商篡改，并基于该信息安全的连接system_config （核心需求！）
- Zone外最大兼容方法：
  - 提供中转服务的善良SN可以正确的将流量转发给ZoneGateway，（对zone_gateway的device did的认证）
  - 当不使用SN的域名时，邪恶的SN提供商无法伪造证书，只能不服务，无法篡改内容
- Zone外先进协议
  - 通过zone_config的owner_did,对zone_config jwt进行验证，确保ZoneConfig是可信的 （不需要，非DNS的DID解析，本身就是去中心的）
  - 解析OwnerConfig的过程通常与解析ZoneConfig分离（不同的DID解析提供商），防止串通作恶的可能性
  - 通过rtcp协议建立不依赖证书的可信连接，确保连上了Zone

域名服务商通过返回另一个zone_config,可以重定向所有的


如果有
```
zone_config = resolve_doc(did_a)
```
则did_a是一个zone_id.
如果zone_config = resolve_doc(did_b),则用did_a,did_b都可以作为zoneid访问同一个zone

## 访问did所拥有的资源
比如，我们知道alic的did是 did_a, 而且知道alic发布了内容,contentid是cid,那么

```
did_host_name = did_a.to_hostname() //to_xxx表示不需要网络行为即可得到结果
if did_host_name.is_same() {
    return http://did_host_name/ndn/cid
} else {
    did_doc = resolve_did(did_a) //did指向了一个OwnerConfig，这比ZoneConfig更精简的
    zone_did = did_doc.get_default_zone()
    zone_host_name = zone_did.to_hostname()
    return http://did_a.zone_host_name/ndn/cid
}
```
可以看到，对于zone_id来说，能不同网络请求得到一个，可以使用zone_host_name就是一个合法的zone_id
通过http协议访问拥有最大兼容性。所谓最大兼容性是系统会保持对该协议的支持，但不是最优的方法
最优的方法系统会一直迭代

我们希望能直接支持, 这样当用户切换其default_zone时，
```
cyfs://did_a/cid
```

## zone_did与zone_hostname
没有SN（或网桥的时候)，很好理解的标准转换
DID::new(“waterflier.com“) == DID::new(“did:dns:waterflier.com“)

有SN的情况

DID:new("waterflier.web3.buckyos.ai") == DID::new("did:bns:waterflier") 
此时，需要通过本地的runtime环境来完成转换
runtime会根据配置， 
if runtime.support_cyfs_dns()  {
    return "waterflier.bns.did"
} else {
    bridge_gateway = get_web3_gateway("bns")
    return waterflier.bridge_gateway
}

域名与did的标准转换
任何did,都可以通过反写的方法变成域名
did:bns:waterflier --> waterflier.bns.did 
但是要能访问 http://waterflier.bns.did/ndn/cid ,则需要域名解析服务器能完整的支持did name 解析协议
    我们开发了支持识别.did的域名解析服务器，其识别过程会读取必要的智能合约状态
    我们鼓励zone内的所有设备，都把默认DNS Server配置成本地网关(cyfs_gateway)



## 系统的3种默认DID

Owner DID, 最不常修改，对长度有严格限制。解析不依赖zone,一般不能通过传统的dns完成解析
Zone DID,不常修改，有精简版本，对长度有严格限制，一般需要通过传统的DNS完成对精简版本的解析,Zone可用的情况下，可以进一步解析得到完整的ZoneConfig
Device DID，常修改，通常是self_auth的。对长度没限制，在得知其所在的zone后，可以通过zone的标准接口进行解析


## Device
Device使用did:dev:$pkx 的格式，是自带公钥的
Device的DID Document通常只保存在Zone，除了用DID标识Device,还可以用device_name@zone_did标识
在Zone内，可以用DeviceName或DevcieDID来建立（可信）链接 （Zone内P2P）
    - 优先使用TCP直连，Zone主要是提供地址查询
    - 有需要的Device会和ZoneGateway keep-tunnel, 通过ZoneGateeway转发可以连上Zone内所有在线的device

在公网上，我们通常不会允许Zone外设备直接连接Device,一般逻辑是先建立到Zone的Tunnel，再基于该Tunnel建立到Device的链接。其stream url如下

```
rtcp://$zone_did/$device_id:1990
```

## Name与DID
DID通常都比较长，具有全网唯一性。
DID太长会在有些地方不是很方便识别。此时可以引入一个更短的唯一标识符Name。在某些确定的环境里会保持唯一性，且更好识别。
name@zone_did 是另一种更短的表达 DID的方式。
在未指明zone_did时，使用name，会有一个确定的逻辑来得到zone_config

在基于zone查询did_docoument的标准方法
- 使用zone_name_provicder,可以基于system_config直接得到
- 使用标准的http方法
    http://{zone_host}/ndn/resolve/{did} 可以查询到
## 一些典型的流程
首先，是去https化

http://waterflier.bns.did/ndn/avator.png 不依赖CA系统(waterflier没有证书)，完成对其语义链接的可信校验
    pk = resolve_auth_key("waterflier.bns.did") // 可以不走域名系统
    assert(DID::from_str("waterflier.bns.did").to_host_name() == "waterflier.web3.buckyos.io")
    resp = wget(http://waterflier.web3.buckyos.io/ndn/avator.png) // 域名提供商或中间人攻击无法伪造返回值
    verify_jwt(resp.path_obj_jwt,pk)
    verfiy_content(resp.body,path_obj_jwt.cyfs_obj_id)

rhttp://buckyos.io/waterflier.bns.did:80/avator.png 在上述流程上，支持不依赖域名系统，与SN建立tunnel,并实现非NDN路径的可信验证
    pk = resolve_auth_key("waterflier.bns.did") 
    zone_doc = resovle_did("sn.buckyos.io") // 可以不走域名系统，走区块链，
    tunnel = create_tunnel(zone_doc) //基于zone_doc，可以不依赖https建立可信链接
    stream = tunnel.open_stream(waterflier.bns.did,80)
    stream.write_http_req
    resp = stream.read //基于rtcp tunnel的通信是可信的，无需验证
    verify(resp,pk)

rtcp://$pubkey.dev.did/:4430  直接连接我的摄像头，获得视频流
rtcp://waterflier.bns.did/mycam1:4430 通过到zone-gateway的tunnel,打开到我的摄像头的视频流


ZoneConfig在boot时候的关键作用
设备启动，拥有node_identiy(只在激活的时创建)

zone_config = resolve_did(node_identiy.zone_did)
veirfy(zone_config,node_identiy.owner_pk,node_identiy.zone_nonce)
verify(node_identiy.devcie_doc,node_identiy.owner_pk)

start_cyfs_gateway(zone_config,device_doc)

//下面这个段落特别的重要 ，决定了“精简版的zone_config应该由哪些字段”
if zone_config.is_ood(node_identify.device_doc) {
    // 链接有固定IP的OOD
    // ood和自己在相同的内网，直接搜索ip地址的端口进行连接
    // ood和自己不在同内网，则必须有SN，通过SN连接
    startup_etcd(zone_config,device_doc)
} else {
    //Zone已经启动，一般来说可以通过 http://zoneid/zone_config 获得full zone config,并得到更详细的信息
    // ood和自己在相同的内网，直接搜索ip地址的端口进行连接
    // ood和自己不在同内网，则必须有SN，通过SN连接
    connect_etcd(zone_config，device_doc)
}

full_zone_config = system_config_client.open("boot/config")
if full_zone_config.is_none() {
    if zone_config.is_first_ood(node_identify.device_doc) {
        full_zone_config = node_identiy.zone_config_jwt
        system_config_client.set("boot/config",full_zone_config)
        do_boot()
    } else {
        // wait boot
    }
} else {
    verify(full_zone_config,owner_pk)
}
print("node boot ok!")
....
service_client = get_service_client(full_zone_config)


对目前的和sn保持链接的逻辑来说，可以不依赖dns来获得更完整的sn zone config
 https://sn_host/zone_config ，在zone的公开device_list中选择一个gateawy通信（有IP地址，实时解析）

 ---> 访问一个zone的服务，应该与哪一个devcie通信的通用问题？

 对能访问full_zone_config的进程来说：

无状态服务：
 1.得到一个服务列表
 2.按评分排序 （综合距离、load，权重等参数）
 3.选择评分最高的

有状态服务 （难以有通用的方法？应该完全交给应用逻辑）
  1. 根据业务逻辑请求计算状态标签
  2. 计算服务中离状态最近的
  3. 服务协议必须支持重定向，选错了后可以告知真实地址

对外部来说
- 使用最大兼容，基于 https://zone_host/kapi/service_name访问
- 集成sdk客户端：基于rhttp://zone_host/kapi/service_name + 返回数据验证 （暂时不支持）

因为服务总是跑在node上的，而node上必定有gateway,所以可以通过gateway作为中间层实现
- 应用服务：基于http://127.0.0.1/kapi/service_name 
- 内核服务: （总是使用最新版本的api),使用和gateway一样的机制，直接连接目标服务 （我们会通过inner service,减少进程间通讯）



