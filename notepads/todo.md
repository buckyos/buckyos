# TODO beta2


## 检查zone_provider的实现 （寻址/服务发现逻辑)
- [ ] 使用 $name.zonehost的方式，得到设备的地址信息，在通内网兼容TCP (later,集群化加入)
- [x] 在resolve_did的环境里，增加通过https协议resolve的设计  : 待测试,有BUG，gateway环境的初始化顺序问题
  - [x] 定义通过https获得zone公开的did_doc的标准 https://$zone_hostname/resolve/$did，$name
  - [x] 支持特定的Name: this_zone, this_device (gateway所在的设备)
- [x] 修改keep_tunnel的流程，能先通过https查询得到必要的doc,再建立链接
- [ ] 结合CYFS Gateway，实现标准的，带有权重的，基于规则的重定向（基于process_chain在下个版本做）

## 用户/公开设备 如何基于现有设施发布DID-DOC

- [ ] 最理想，通过BNS合约 ---> 我们是否应该使用成熟的合约？
- [x] 在DID-Doc中，增加标准的，通过https协议查询最新版本的ServiceEP

