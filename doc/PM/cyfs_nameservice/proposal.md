 NameService是网络服务基础功能，通过它用户可以用方便记忆的字符串来访问网络上的服务，比如dns让用户可以直接用域名来访问网络服务，不需要再去记忆非常难记住的ip地址。本项目提供了一个比dns更优秀的名字服务，它支持多种通信协议地址查询，支持p2p查询，支持安全查询等。

该项目主要功能如下：

1. 每一个设备以域名规则命名，分为zone名和设备名，组合成设备完整名字，比如zone名为example.zone，设备名为test，则完整名字为test.example.zone，同zone内的设备zone名必须相同。
2. 自动发现zone内所有设备，并能自动更新zone内设备配置，每个设备成员使用都通过身份证书标识自身，并且通信通过该证书加密，根据底层实现不同采用的证书可以是tls、cyfs、eth等形式，详细见下文provider介绍。
3. 节点信息来源支持多种形式 （每种形式为一个provider），比如通过dns、eth链、etcd等，用户可以扩展新的来源形式。通过配置用户可以设置本地支持多少种来源以及每种来源的优先级，程序内部根据优先级逐一从provider获取名字信息直到找到为止，如果遍历所有provider也找不到则返回查找失败。同一个产品应该使用相同的provider配置，防止出现不同的provider。
4. 提供根据设备名字或zone名获取节点信息接口，节点信息包含Zone外信息和Zone内信息，接口可以设置是查询所有信息，还是只查询zone内或zone外信息，Zone外信息包含必要的连接信息，比如ip或cyfsid等，Zone内包含产品自定义信息，自定义信息支持加密。如果输入是zone名时，将获得同zone内所有节点信息。
5. 提供通过设备名字获得设备证书接口，证书支持cyfs、tls证书等格式。
6. 支持dns服务接口，方便现有的基于ip协议的服务接入。

**Provider介绍**

Simple DNS Provider：

该provider只是根据名字直接从外网DNS查询相关信息，主要用于提供节点外zone信息，规则如下：

1. 直接根据名字从外网DNS查询名字的txt记录，txt信息格式为：protocol://xxxx，protocol字段标识通信协议，如ip通信为ip://x.x.x.x，cyfs协议通信为cyfs://xxx。如果ip通信也可以为域名配置A记录。
2. 如果是ip通信节点身份信息必须为可验证根证书颁发的tls证书，如果是cyfs通信则节点采用cyfs身份证书。
3. 这种provider不能提供完备的nameservice功能，它只提供单节点的连接信息查询，必须要其它provider配合，比如ETCD provider。

Simple ETH Provider：

该provider只是根据名字直接从ETH合约中查询相关信息，主要用于提供节点外zone信息，规则如下：

1. 提供名字服务注册售卖合约，合约的拥有者可以更新名字相关信息，信息格式为：protocol://xxxx，protocol字段标识通信协议，如ip通信为ip://x.x.x.x，cyfs协议通信为cyfs://xxx。
2. 直接根据名字从ETH链上查询名字的地址记录。
3. 如果是ip通信节点身份信息必须为可验证根证书颁发的tls证书，如果是cyfs通信则节点采用cyfs身份证书。
4. 这种provider不能提供完备的nameservice功能，它只提供单节点的连接信息查询，必须要其它provider配合，比如ETCD provider。

ETCD Provider：

该provider是从etcd中获取节点信息，主要用于提供zone内节点信息。如果要使用该provider需要解决etcd启动问题，启动了etcd之后所需数据之间从etcd中查询就行了

DNS+P2P Provider（暂未实现）：

该provider是根据zone名从外网DNS查询到Zone接入节点信息，再通过p2p的形式获取到整个zone节点信息，规则如下：

1. 构造zone.${zone name}，如果zone名为example.zone，则查询域名为zone.example.zone，域名配置txt记录，txt信息格式为：protocol://xxxx，protocol字段标识通信协议，如ip通信为ip://x.x.x.x，cyfs协议通信为cyfs://xxx。如果ip通信也可以为域名配置A记录。如果Zone入口支持多个节点，txt记录以；分隔，A记录则为多个ip。
2. 如果是ip通信节点身份信息必须为可验证根证书颁发的tls证书，如果是cyfs通信则节点采用cyfs身份证书。
3. 此种模式下同Zone所有节点必须采用相同的通信协议。
4. 节点之间通过raft协议维护zone内各节点信息的同步。
5. 该provider提供nameservice的完备功能

ETH+P2P Provider（暂未实现）：

该provider是根据zone名从外网DNS查询到Zone接入节点信息，再通过p2p的形式获取到整个zone节点信息，规则如下：

1. 提供名字服务注册售卖合约，合约的拥有者可以更新zone名相关信息，信息格式为：protocol://xxxx，protocol字段标识通信协议，如ip通信为ip://x.x.x.x，cyfs协议通信为cyfs://xxx。如果Zone入口支持多个节点，txt记录以；分隔。
2. 根据zone名从ETH上查找zone入口节点信息。
3. 如果是ip通信节点身份信息必须为可验证根证书颁发的tls证书，如果是cyfs通信则节点采用cyfs身份证书。
4. 此种模式下同Zone所有节点必须采用相同的通信协议。
5. 节点之间通过raft协议维护zone内各节点信息的同步。
6. 该provider提供nameservice的完备功能

