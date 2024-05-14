# quick start
请注意整个demo都没有做任何的身份验证，仅限于测试



## 获得BuckyOS booter(node_daemon)
方法一、 在使用时通过docker pull得到
方法二、 自己build git clone后build得到

buckyos镜像里包含了buckyos的kernel组件，以及一系列基本的cli工具。


## 构造Zone Config并发布 
准备工作：1个可用的域名

BuckyOS的booter在启动时，需要读取保存在去中心的（非公司经营）的基础设施上的集群配置信息。我们称作zone_config. zone_config的具体内容和集群的物理拓扑有关，理解原理后可以自行编写。我们针对典型集群，已经提供了预配置的zone_config
2and1_zone_config.json是我们要用的

使用下面命令
```
buckytool -gen_zone_config 2an1_zone_config.json
```
会得到base58编码的文本。
通过你的dns注册商的工具，新建一个text record并保存上述结果。


## 准备2+1台主机，其中1台主机是有公网IP的VPS
0. ssh到目标主机
1. 修改机器名，与zone_config中的相符
2. 通过工具基于zone的私钥得到机器的私钥 (demo版无身份认证，此步骤跳过)
3. 使用下面命令拉取bucky booter docker镜像并启动(如果是自己build的docker镜像则替换成自己的镜像路径)
```
docker pull buckyos/buckyos
docker run -d --name buckyos --restart=always -v /etc/bucky:/etc/bucky -v /var/run/bucky:/var/run/bucky -v /var/log/bucky:/var/log/bucky buckyos/buckyos
```

也可根据发行版的不同，将node_daemon变成系统的默认服务，比如ubuntu:
```
```
### 新集群检查etcd的状态
异构网络的etcd的连通问题
在内网的etcd1,etcd2可以直接使用url访问其他的两台etcd
运行在gateway上的etcd需要通过本地代理来访问其他的etcd,分别是 http://127.0.0.1:port1 和 http://127.0.0.1:port2
port1,port的配置

完成上述工作后，集群里的etcd将会启动，可以通过下面命令检查etcd的状态



## 启动booter 

## 分支一、通过bucky toolkit进行首次配置并查询状态

app/service config

node_config是根据app config和zone内的实际物理拓扑通过调度器构造出来的。demo并未包含调度器，因此该文件需要手工编写。
如果前面选择的都是典型配置，那么就可以使用我们预设的3个node config

### 理解 app config中目前的内容
0. bakcup 
1. dfs服务的endpoint list
2. smb服务的endpoint
3. gateway的配置，核心在于
反连的端口配置

### 写入config到etcd
buckyos 的booter在一个新集群启动后，是不会有任何行为的，需要让上述配置生效后才能让集群里的各种服务生效
使用下面命令写入配置
```
buckytool --status
buckytool --config_app gateway_service.json
buckytool --config_node etcd1_node_config.json
buckytool --config_node etcd2_node_config.json
buckytool --config_node gateway_node_config.json
```


## 分支二、通过bucky toolkit查询恢复进度
因为zone config中配置了backupserver,因此booter在启动后，会自动的从backup server上恢复etcd配置，进而恢复全部配置和数据


## 在同局域网使用smb服务访问dfs
根据app config,smb服务的端口在etcd1服务器上
通过\\etcd1\ 可以访问dfs


## 通过http://dfs.yourdomainname 只读的访问文件系统



## 通过bucky toolkit查看备份情况

## 一些常见的故障自动修复逻辑
经过这只是一个demo,不过基于etcd和dfs的能力，我们已经拥有了一个基本可靠的系统，并拥有了一定的容灾能力