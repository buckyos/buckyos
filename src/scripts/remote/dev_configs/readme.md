# dev_config目录介绍

该目录下保存了一个典型的“分布式测试环境“所需要的全部信息和配置。目录结构如下

## 根目录
- machine.json OK 
- ca证书 OK 


## owners目录
该目录下存放用户配置和对应的zone配置
虽然允许一个用户有多个zone,但目前大部分情况下还是1对1的配置
userid目录下的文件复制到.buckycli目录下后可用root权限

### $userid
- user_config.json
- user_private_key.pem
- zone_config.json
- zone_boot_config.txt
- hosts cyfs_gateway可以加载的域名配置文件，包含了需要配置的dns txt字段
  - DID 
  - PX0
  - PX1 

## $node_id 目录

该目录下的文件，会在更新配置阶段，复制到 对应虚拟机的 /opt/buckyos/目录
目前有两类node

### ood（已经激活）
- /etc/node_identity.json 
- /etc/node_private_key.pem 
- /etc/device_config.json 这个是node_idntity的一部分,单独拿出来只是方便查看？
- /etc/machine.json 配置了btest网桥

### sn node比较特殊
- /data/web3_gateway/sn.db 数据库
- /etc/web3_gateway.yaml 核心的配置文件
- /etc/device_config.json
- /etc/node_private_key.pem
- /etc/sn.$sn_host 的tls证书
- /etc/*.web3.$sn_host的tls证书
- /etc/machine.json 配置了btest网桥

## 上述文件的构造顺序
- 先构造根目录下的文件 create_ca,machine.json是固定的
- 然后构造owners create_user_config,create_zone,create_cert
- 随后后构造node_id下的配置 create_device_config,create_ood_configs,copy cert from owners_foder
- 最后构造sn（特别是sn.db) 