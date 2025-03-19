# 需求
使用Rust开发一个在局域网部署的DNS服务器，支持以下功能：
- 基于trust_dns_resolver或其它主流的DNS库
- 能支持主流的DNS扩展，包括DNS2，DNS@https等
- 能够通过provider模式，扩展不同的解析器实现，并可通过配置文件挑战处理顺序
- 基于上述模式，支持加载标准的host文件
- 基于上述模式，支持加载自定义的host文件，自定义的host文件除了支持A记录外，还支持TXT记录
- 基于上述模式，支持1，2个主流的D-DNS协议，允许认证用户更新其子域名的A记录和TXT记录。
- 能根据请求来源，进行不同的解析
- 可以配置fallback的DNS服务器，当本地解析失败时，可以fallback到指定的DNS服务器

## zone provider需求
- 知道当前zone_config(目前只能支持一个zone)
- 根据zone_config,读取system_config,得到  $device_name.devices.$zone_id 的地址
- 根据zone_config,读取system_config,得到  $service_name.$zone_id 的地址

## ddns (web2.5)
- 解析zone-id到 公网gateway上（固定解析）
- 解析zone-id到 D-DNS的当前结果上
- 通过D-DNS的API，更新zone-id的A记录和TXT记录，TXT记录按需更新，主要是更新ZoneConfig
- 要设计一个简单的D-DNS存储器接口，分离存储器和D-DNS的实现

## zone内服务发现的依赖问题
1. 通过ZoneConfig，只能知道OOD的名字，并不知道地址，如何得到system_config_service的地址？

可用的工具
用户有一个可以控制的域名？（必须有）
用户是否有公网的节点
内网是否支持穿透
内网是否支持传统的网络发现

### OOD 的激活启动
如果是单节点非常简单
如果是多节点，那么在知道其它OOD的Name的情况下
1. lookup($oodname.devices.$zone_id)，得到地址,需要Web2.5支持
2. 在局域网ping $oodname, 得到地址，需要局域网的DHCP服务器呢DNS支持，并且
3. OOD在局域网的自定义广播协议？
4. 如果OOD在公网怎么办：等待内网的OOD来连自己，

### 已经激活过的Device，很久没有启动，再次启动后如何找到ZoneSytemConfig注册自己？
a. 通过上次启动成功的信息继续
b. 可以可靠依赖的Zone外基础设施：
    DNS / 区块链系统（关键），这些系统的更新成本很高，所以应尽量避免写入需要频繁更新的信息
    当前网络的广播能力，局域网广播成本低速度快，应优先使用。但可能不可靠
    不使用传统的，基于DHT的广域网广播。DHT是区块链系统的Booter,不应该给应用系统用

==> DNS / 区块链系统（关键），这些系统的更新成本很高，所以应尽量避免写入需要频繁更新的信息
a. OOD 的名字,如果 OOD有公网固定IP，那么可以写入该地址
b. 不应写入OOD的非固定IP，这个变化太快了
    此时Zone应该使用D-DNS的方式，通过域名解析来得到实时的地址.此时会潜在的依赖一个外部服务 

结论：用户将ZoneConfig保存在区块链上，其中最好包含一个固定IP（此时对外的依赖最少，且更新的成本最低）
      Web2的过渡用户，在ZoneConfig中写入一个外部服务域名，该域名可能支持 备份、D-DNS以及Web2.5，可以通过简单的通用协议，给用户选择供应商的自由