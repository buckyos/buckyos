
## 根据典型流程，检查关键的配置文件修改 

- [x] 调整krpc获得服务地址的方法
- [x] 区分ZoneConfig和ZoneBootConfig
- [x] gateway/system_config/schedule能基于ZoneBootConfig初始化
  - [x] gateway
  - [x] schedule
  - [x] system_config
- [x] 正确实现did<->host_name的转化： 要支持读取配置文件修改
- [x] DNS 能正确的resolve zone-boot-config,SN能正确通过NameInfo返回Zone-Boot-Config
- [x] 检查Node_Active构造的配置文件
- [ ] 修改SDK,注意修改SDK中的相关config数据结构定义
- [x] 修改所有配置文件，和DNS配置
- [x] XXXConfig与现有的DIDDoc体系进行最大程度的兼容设计
- [x] 通过配置文件切换bns web3网桥
- [x] device_info应该是device_doc的扩展

## 检查系统内权限是否能正确与DID集成
- [ ] 可同时使用did和友好名称(name)
- [ ] users/$username/config -> OwnerConfig
- [ ] 导入User(Owner) /Config时，需要对name的一致性进行检查
- [ ] 检查device的注册与上报逻辑，区分私有设备和公开设备
- [ ] review现有权限配置，更新SUDO文档
- [ ] 增加SDK里的verify-hub api
    从请求中提取session_token
    从session_token中提取出user_id,app_id
    如果请求中有host信息，则根据host信息比较app_id
    根据业务，判定action和target resource

- [] session_token的构成
    JWT型，验证方只需要验证是否是由可信公钥签名即可
        根据环境变量由基础的 get_basic_trust_public_key 
        verify-hub的通用接口 get_trust_public_key(kid),除了verify_hub,kid应与username相同
    
        通过支持resolve_did，可以允许跨系统更换公钥
    现在为了简单，过期时间都很长
    未来应该改成 向verify-hub注册一个临时私钥的形式


## 检查zone_provider的实现 （寻址/服务发现逻辑)
- [ ] 使用 $name.zonehost的方式，得到设备的地址信息，在通内网兼容TCP (later,集群化加入)
- [x] 在resolve_did的环境里，增加通过https协议resolve的设计  : 待测试,有BUG，gateway环境的初始化顺序问题
  - [x] 定义通过https获得zone公开的did_doc的标准 https://$zone_hostname/resolve/$did，$name
  - [x] 支持特定的Name: this_zone, this_device (gateway所在的设备)
- [ ] 修改keep_tunnel的流程，能先通过https查询得到必要的doc,再建立链接
- [ ] 结合CYFS Gateway，实现标准的，带有权重的，基于规则的重定向（基于process_chain在下个版本做）

## 用户/公开设备 如何基于现有设施发布DID-DOC

- [ ] 最理想，通过BNS合约 ---> 我们是否应该使用成熟的合约？
- [x] 在DID-Doc中，增加标准的，通过https协议查询最新版本的ServiceEP

## 将SN，可信发行商，默认源切换到buckyos.ai

- [ ] 修改配置，默认使用.ai域名
- [ ] 打出安装包，可正常完整安装和激活
- [ ] 正常安装的包，能触发自动更新


## BUGS

- [ ] 打包的时候分离默认应用和系统组件的打包
- [ ] 通过pkg+tar的方法导入应用镜像似乎有问题
- [ ] SystemConfig首次启动无法识别verify-hub身份的问题-->通用的认证库（可信密钥初始化）