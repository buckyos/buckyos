# quick start
请注意整个demo都没有做任何的身份验证，仅限于测试
demo目前面向的是能熟练使用Linux cli和docker的高级用户 ，面向普通用户我们会提供更加友好的GUI工具来完成buckyos的部署。


## 获得BuckyOS booter(node_daemon)
方法一、 在使用时通过docker pull得到

```bash
docker pull buckyos/buckyos

```

方法二、 自己build git clone后build得到

buckyos镜像里包含了buckyos的kernel组件，以及一系列基本的cli工具。


## 构造Zone Config并发布 
准备工作：1个可用的域名

BuckyOS的booter在启动时，需要读取保存在去中心的（非公司经营）的基础设施上的集群配置信息。我们称作zone_config. zone_config的具体内容和集群的物理拓扑有关，理解原理后可以自行编写。我们针对典型集群，已经提供了预配置的zone_config
2and1_zone_config.json是我们要用的

使用下面命令
```bash
docker pull buckyos/buckycli
docker run --rm buckyos/buckycli --dump_text 2an1_zone_config.json
```
会得到base58编码的文本。通过你的dns注册商的工具，新建一个text record并保存上述结果。


## 准备2+1台主机，其中1台主机是有公网IP的VPS
0. ssh到目标主机
1. 修改机器名，与zone_config中的相符（内网的2台是etcd1,etcd2,公网的是gateway）
2. 通过工具基于zone的私钥得到机器的私钥 (demo版无身份认证，此步骤跳过)
3. 使用下面命令拉取bucky booter docker镜像并启动(如果是自己build的docker镜像则替换成自己的镜像路径)

```bash
docker pull buckyos/buckyos
```
因为不同的主机的id不同，所以其node_identity的配置也不同。因此我们需要在不同的主机上用正确的身份配置来启动buckyos(booter)

在etcd1上：

```bash
docker run -d --name buckyos --restart=always -v /etc/bucky:/etc/bucky -v /var/run/bucky:/var/run/bucky -v /var/log/bucky:/var/log/bucky buckyos/buckyos
```

在etcd2上：

```bash
docker run -d --name buckyos --restart=always -v /etc/bucky:/etc/bucky -v /var/run/bucky:/var/run/bucky -v /var/log/bucky:/var/log/bucky buckyos/buckyos
```

在gateway上:
```bash
docker run -d --name buckyos --restart=always -v /etc/bucky:/etc/bucky -v /var/run/bucky:/var/run/bucky -v /var/log/bucky:/var/log/bucky buckyos/buckyos
```

## 设置buckyos 启动运行
如果有需要，也可根据发行版的不同，将node_daemon变成系统的默认服务，比如ubuntu:
```
```

### 新集群检查etcd的状态
异构网络的etcd的连通问题
在内网的etcd1,etcd2可以直接使用url访问其他的两台etcd
运行在gateway上的etcd需要通过本地代理来访问其他的etcd,分别是 http://127.0.0.1:port1 和 http://127.0.0.1:port2
port1,port的配置

完成上述工作后，集群里的etcd将会启动，登陆上述3台主机中的任意一台，可以通过下面命令检查etcd的状态

```bash
docker run --rm buckyos/buckycli --init $your_domain_name
docker run --rm buckyos/buckycli --config_get "/systeminfo"
```
如果该命令返回了json text,并且没有错误信息，那么说明etcd已经启动并且正常工作了。

## 启动buckyos上的service
完成上述流程后，系统里启动了一个etcd service,并且每台服务器上都已经运行了buckyos的booter。此时系统处于空转状态，没有任何一个有意义的服务在运行。下面我们将通过在etcd中写入一些配置，来启动一些重要的基础服务。

### 分支一、通过buckycli进行首次配置并查询状态

app/service config

node_config是根据app config和zone内的实际物理拓扑通过调度器构造出来的。demo并未包含调度器，因此该文件需要手工编写。本文并不打算详细介绍手工编写node_config的过程，因此
如果前面选择的都是典型配置，那么就可以使用我们预设的3个node config。

#### 写入node config到etcd

使用下面命令写入配置
```
docker run --rm buckyos/buckycli --config_write "/app_services/gateway" --file gateway_service.json
docker run --rm buckyos/buckycli --config_write "/nodes/etcd1" --file etcd1_node_config.json
docker run --rm buckyos/buckycli --config_write "/nodes/etcd2" --file etcd2_node_config.json
docker run --rm buckyos/buckycli --config_write "/nodes/gateway" --file gateway_node_config.json
```

#### 选读:理解 app / node config中目前的内容
0. bakcup 
1. dfs服务的endpoint list
2. smb服务的endpoint
3. gateway的配置，核心在于
反连的端口配置

### 分支二、通过buckycli查询恢复进度
因为zone config中配置了backupserver,因此buckyos booter在启动后，会自动的从backup server上尝试恢复etcd配置，进而恢复全部配置和数据。
也就是说，如果已经有了一个完整的集群，在新的3台主机上运行buckyos booter后，集群的状态会自动的恢复到上一个备份点。

有的时候恢复数据可能需要较长的时间，通过下面命令可以查询恢复task的进度

```bash
docker run --rm buckyos/buckycli --backup_list_task 
```

## 在同局域网使用smb服务访问dfs

根据app config,smb服务的端口在etcd1服务器上
通过下面路径可以访问dfs

```
\\etcd1\
```

相比NAS，通过在zone内增加新的节点就可以增加该分布式文件系统的容量，并且基于DFS的基础特性，保存在DFS上的文件冗余度更高。我们推荐的典型配置支持：
1. 损坏任意一块硬盘都不会丢失数据
2. 尽力可用，当损坏扩大时，系统首先是不可安全写入，随后会尽量让数据可读。我们知道数据是每个人最宝贵的资产，因此buckyos会尽力让数据可用。

##　通过http协议访问DFS
因为我们配置了公网的gateway,因此我们可以通过下面的url只读的浏览文件系统。请再次注意这是一个DEMO，没有做任何的权限控制，请仅仅用于测试目的。
```
http://$your_zone_domain/dfs
```
要注意的是，记得在您的域名管理面板将$your_zone_domain指向gateway主机的IP.


## buckyos的备份

### 通过buckycli查看备份情况

通过下面命令可以看到
```bash
docker run --rm buckyos/buckycli --backup_list_all_task 
```

## 一些常见的故障自动修复逻辑
经过这只是一个demo,不过基于etcd和dfs的能力，我们已经拥有了一个基本可靠的系统，并拥有了一定的容灾能力.
我们知道，任何cluster都会发生故障，并且需要运维。但对普通人来说，没有办法进行复杂的运维操作，甚至可能通过运维操作会进一步的损害系统。因此buckyos在设计上，希望应对系统的故障有固定的套路，让没有技术背景的终端用户，也能完成cluster的故障运维。

1. 理解系统的状态。 
- 蓝色，系统非常安全。所有的数据都已经完成了备份，任何损坏几乎都是可以修复的。对一个频繁工作的系统，通常难以长期稳定在蓝色状态。
- 绿色，系统正常工作（也是最常见的状态）。系统没有任何故障，但是由于备份速度的原因，有一部分数据还未完整备份。因此在极端情况下，系统有概率丢失一部分最新的数据
- 黄色，系统有问题但依旧大部分功能可用，需要尽快处理。最常让系统进入黄色状态的情况是一块硬盘损坏或一台机器损害。
让系统由黄变绿是普通人要及时完成的事情，通常来说只有两个动作
1. 更换损坏的硬盘，比如如果系统里有可更换硬盘的设备，且已经明确的知道了是哪一块硬盘损坏，那么只需要简单更换另一个容量大于等于损坏的硬盘即可
2. 更换损坏的机器，对于不可拆卸的机器发生问题，只需要简单的送修或更换新设备即可。
3. 丢弃损坏的机器，并加入一台能力大于等于损坏机器的新机器。需要在控制面包中进行一些对应的设备管理操作。

以上操作完成后，系统并不会立刻由黄转绿，而是需要等待新的硬件或设备完成状态恢复。
- 橙色，系统故障。基本进入只读状态，大部分功能不可用，可能已经损失了部分数据。同时损害多台设备（包括系统在黄色后再次损坏设备）会让系统进入橙色状态。修复橙色故障的逻辑和修复黄色一致，但可能需要更久的自动恢复时间，并且有可能无法恢复全部的数据。
- 红色（不可用），系统完全故障。