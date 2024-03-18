 NameService是网络服务基础功能，通过它用户可以用方便记忆的字符串来访问网络上的服务，比如dns让用户可以直接用域名来访问网络服务，不需要再去记忆非常难记住的ip地址。本项目提供了一个比dns更优秀的名字服务，它支持多种通信协议地址查询，支持去中心化查询，支持安全查询，查询到地址信息的身份校验等。

##### 功能：

1. 每一个设备以域名规则命名，分为zone名和设备名，组合成设备完整名字，比如zone名为example.zone，设备名为test，则完整名字为test.example.zone，同zone内的设备zone名必须相同。
2. zone和设备都有各自的身份证书，身份证书和名字关系可以校验。
3. 自动发现zone内所有设备，并能自动更新zone内设备配置，设备之间加密通信，防止传输过程中被篡改。
4. 设备信息除了连接基本信息之外，用户还可以写入自定义的设备信息，并且还可以指定信息是zone内可见，还是zone外可见，是否加密等。
5. 设备信息来源支持多种形式 （每种形式命名为一个provider），比如通过dns、eth链、etcd等，用户也可以扩展新的来源形式。通过配置用户可以设置本地支持多少种来源以及每种来源的优先级，程序内部根据优先级逐一从provider获取名字信息直到找到为止，如果遍历所有provider也找不到则返回查找失败。同一个产品应该使用相同的provider配置，防止出现不同的provider。

##### 用户接口：

用户接口只允许通过http://127.0.0.1:3453访问，不支持跨机器访问

1. 获取节点信息

   http://127.0.0.1:3453/resolve?

   请求方式：GET

   请求参数：

   ​	name：string，请求节点的名字

   ​	type：number，1 地址信息、2 扩展信息、3 全部信息 

   返回格式：

   ```json
   [{
       "name": "device name",
       "addr_info"： {
       	"protocol": "string，连接协议，可取值：ipv4、ipv6、cyfs",
       	"address": "string，不同的连接协议有不同的值",
   	},
   	"extend": {
           "extend_key": {
    			"extend_value": "",
    			""
    		}
       }
   }]
   ```

   

2. 获取节点证书

   http://127.0.0.1:3453/cert?

   请求方式：GET

   请求参数：

   ​	name：string，请求节点的名字

   返回格式：

   ```json
   {
       "name": "device name",
       "cert_type": "string，证书格式，可以取值：x509",
       "cert": "string，证书字符串，根据证书类型有不同的编码方式，x509类型时为原证书内容"
   }
   ```

   

3. 设置节点扩展信息

   http://127.0.0.1:3453/extend

   请求方式：POST

   请求参数：

   ```json
   [{
       "extend_key": "string，扩展数据名字",
       "extend_value": "string，扩展数据",
       "is_encrypt": "bool，true数据加密，false数据不加密",
       "scope": "string，数据作用范围，zone-in：只有zone内可访问，zone-out：zone外可以访问"
   }]
   ```

   

4. 删除节点扩展信息

   http://127.0.0.1:3453/extend_del

   请求方式：POST

   请求参数：

   ```json
   ["extend_key"]
   ```

   

5. 

##### 已有Provider介绍

Simple DNS Provider：

该provider只是根据名字直接从外网DNS查询地址信息，并且将连接查询到的地址信息，获取对端身份证书，主要用于提供节点外zone信息，规则如下：

1. 直接根据名字从外网DNS查询名字的txt记录，txt信息格式为：protocol://xxxx，protocol字段标识通信协议，包括ipv4,ipv6,cyfs。
2. 根据从dns上查询到的信息，连接节点获取身份证书。如果是ip通信节点身份信息必须为可验证根证书颁发的tls证书，如果是cyfs通信则节点采用cyfs身份证书。
3. 这种provider不能提供完备的nameservice功能，它只提供单节点的连接信息查询，必须要其它provider配合，比如ETCD provider。

Simple ETH Provider：

该provider只是根据名字直接从ETH合约中查询相关信息，主要用于提供节点外zone信息，规则如下：

1. 提供名字服务注册售卖合约，合约的拥有者可以更新名字相关信息，信息包括：

   ​	protocol://xxxx，protocol字段标识通信协议，包括ipv4,ipv6,cyfs。

   ​	certificate：身份证书。

2. 直接根据名字从ETH链上查询名字的地址记录。

3. 如果是ip通信节点身份信息必须为可验证根证书颁发的tls证书，如果是cyfs通信则节点采用cyfs身份证书。

4. 这种provider不能提供完备的nameservice功能，它只提供单节点的连接信息查询，必须要其它provider配合，比如ETCD provider。

ETCD Provider：

该provider是从etcd中获取节点信息，主要用于提供zone内节点信息。如果要使用该provider需要解决etcd启动问题，启动了etcd之后所需数据之间从etcd中查询就行了

DNS+Decentralization Provider（暂未实现）：

该provider是根据zone名从外网DNS查询到Zone接入节点信息，再通过p2p的形式获取到整个zone节点信息，规则如下：

1. 为zone名配置txt记录，如example.zone，txt信息格式为：protocol://xxxx，protocol字段标识通信协议，包括ipv4,ipv6,cyfs。如果Zone入口支持多个节点，txt记录以；分隔。
2. 此种模式下同Zone所有节点必须采用相同的通信协议。
3. 节点之间通过raft协议维护zone内各节点信息的同步。
5. 该provider提供nameservice的完备功能

ETH+Decentralization Provider（暂未实现）：

该provider是根据zone名从外网DNS查询到Zone接入节点信息，再通过p2p的形式获取到整个zone节点信息，规则如下：

1. 提供名字服务注册售卖合约，合约的拥有者可以更新zone名相关信息，信息格式为：protocol://xxxx，protocol字段标识通信协议，如ip通信为ip://x.x.x.x，cyfs协议通信为cyfs://xxx。如果Zone入口支持多个节点，txt记录以；分隔。
2. 根据zone名从ETH上查找zone入口节点信息。
3. 此种模式下同Zone所有节点必须采用相同的通信协议。
4. 节点之间通过raft协议维护zone内各节点信息的同步。
5. 该provider提供nameservice的完备功能

