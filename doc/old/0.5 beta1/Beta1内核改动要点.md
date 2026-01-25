# Beta1内核改动要点

## 完善resolve-did机制

- 如果did本身是nameobj-id,那么立刻验证 （did:dev:$devpubkey)（此时没有fregment)

```
从任意渠道得到doc
用来自did里的公钥验证
保留iat最新的那个（没有iat用exp-默认过期时间得到iat)
```

- 如果fragment是owner,那么只认root_trust_provider的结果或本地缓存结果里最新的那个
```
从本地trust_provider得到did-doc
从root_trust_provider得到did-doc
    root_trust_provider和did.method绑定，目前有2个
        bns->bns_provider,未来支持合约
        web->https_provider,通过https协议进行验证(cyfs协议的一部分)
比较，得到最新的那个
更新本地缓存(本地的root_trust的更新需要不同的root_trust_provider可信等级)
    只有可信等级更高的结果，才能刷新本地的存储（ https是1，本地缓存是2，本地root_trust是0，基于智能合约（去中心基础设施）的是0）
```

- 其它类型的解析,走标准验证流程
```
从任意provider得到did-doc (可能会有多个)
    did-doc = provider.resolve_did(target_did,fragment)
    auth-doc1  = provider.get_auth(target_did) or auth-did = did-doc.get_auth()
    auth-doc2 = resolve_did(auth-did,"owner")
    校验(did-doc,auth-doc.public_key)

返回iat最新的那个
更新本地缓存
```

## pkg逻辑调整
- 是在制作安装包之前调用buckycli install_pkg,还是在安装时，执行buckycli install_pkg?
    原理上，还是在目标环境安装更可靠一些，缺点是要带上buckycli
- bin目录永远不enable符号？
    - 缺点：安装时的删除在windows下可能不会成功？用重命名逻辑？还是要处理GC
    - 好处：防止pkgs目录膨胀（不用处理GC）
    - 好处：不用处理“因为系统原因无法enable 符号的两种逻辑”（回退到最保守模式）
    - 缺点：强制要求pkg每次都更新版本号，否则可能会有潜在的bug
- 对pkg升级流程进行独立验证
在需要升级的env，或parent env执行
```
buckycli pkg_sync #从源拉最新版本的meta-db
buckycli pkg_upgrade [$pkg_id] 
```


## 切换到新版本的cyfs-gateway 上来
cyfs-gateway process-chain机制提供了巨大的灵活性，要仔细设计其目标构成

### 在新结构上，正确实现service selector
使用cyfs-gateway的基础设施实现，而不是简单的去扩展命令




### node-gateway / zone-gateway 处理应用访问时的权限管理
应用服务=> 判断来源app,并决定是否允许访问目标服务 app 
浏览器=> 判断host,并决定最终的路由目的地，以及可以在域内使用的服务

## schedule逻辑完善

### 彻底完成调度算法框架（在算法层面已经是分布式的了）
- 反复添加/删除，删除能彻底
- 针对动态资源(主要是端口号)的分配与确认
- 由于DFS的默认存在，所以不存在app是有状态的
- 只有服务才有Local data的使用能力，app只能使用local-cache
- 根据node-instance的需要进行调度
- 迁移(OPTask)
    - 普通迁移逻辑：先分配 再释放？ / 尽量完成目录复制？
    - DFS迁移
    - ETCD迁移

### boot schedule不是从模版文件创建，而是基于标准逻辑添加实体(对象)

### scheduler 调度结果单文件化（模块化）
在内地运行的cyfs-gatway看来，其配置文件是完整且模块化的，可以通过简单的删除某个文件完整的去除一个实体的影响
TODO:cyfs-gateway的配置文件include/更新机制 需要提供对remote sync的支持

## 正确实现ood boot阶段的cyfs-gatway逻辑
根本目的，是支持system_config（调度器无法管理）
- Boot阶段最重要的就是提供到system_config的支持

### Node的boot流程
- 努力连接上system_config,并根据需要调整boot_config的配置
- 


### 单OOD + WLAN Gateway系统
- OOD需要和Gateway keep tunnel(此时应该是没有SN的)
- Gateway上运行ZoneGateway,因此保存证书（获取后保存在system config里）。
- 这种情况肯定不用SN 转发，但还是需要D-DNS
- 只有域名时，Gateway支持ZoneDNSProvider?配置NS记录？

### 多OOD系统里的OOD
- OOD先与SN/Gaeway Keep tunnel
- 随后尝试与所有的OOD都建立连接（直连/中转），此时要根据etcd的需要来安排boot



### 纯Gateway节点

### rtcp在中转时可以选择保持身份（相当于rtcp帮助中转tls）
