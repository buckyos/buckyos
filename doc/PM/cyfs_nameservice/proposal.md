 NameService是网络服务基础功能，通过它用户可以用方便记忆的字符串来访问网络上的服务，比如dns让用户可以直接用域名来访问网络服务，不需要再去记忆非常难记住的ip地址。本项目提供了一个比dns更优秀的名字服务，它支持多种通信协议地址查询，支持去中心化查询，支持安全查询，查询到地址信息的身份校验等。

##### 功能：

1. 每一个名字以域名规则命名，分为zone名和服务名，比如zone名为example.zone，服务名为test，则完整名字为test.example.zone，同zone内的所有服务zone名必须相同。
2. 名字代表的服务包括zone内的节点设备，每个节点设备都有自己的身份证书，保证从节点获得的信息安全可信。
3. 名字信息除了连接基本信息之外，用户还可以配置自定义信息，并且还可以指定信息是zone内可见或zone外可见。
4. 名字信息来源支持多种形式 （每种形式命名为一个provider），比如通过dns、eth链、etcd等，用户也可以扩展新的来源形式。通过配置用户可以设置本地支持多少种来源以及每种来源的优先级，程序内部根据优先级逐一从provider获取名字信息直到找到为止，如果遍历所有provider也找不到则返回查找失败。同一个产品应该使用相同的provider配置，防止出现不同的provider。

##### 用户接口：

用户接口只允许通过http://127.0.0.1:3453访问，不支持跨机器访问

1. 获取名字信息

   http://127.0.0.1:3453/resolve?

   请求方式：GET

   请求参数：

   ​	name：string，请求信息的名字

   返回格式：

   ```json
   {
       "name": "service name",
       "type": "zone|node|service"
       "addr_info"： [{
       	"protocol": "string，连接协议，可取值：tcp、https、cyfs",
       	"address": "string，不同的连接协议有不同的值",
       	"port": "端口号"
   	}],
   	"api_version": "v1,服务api版本",
   	"extend": {
           "extend_key": {
    			"extend_value": "",
    		}
       }
   }
   ```

   

2. 获取名字证书

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

3. 

节点之间通信接口：

以下接口都通过https访问

1. 获取指定名字的身份证书，该接口没有权限限制，谁都可以获取
2. 获取指定名字的信息，该接口请求时必须带上请求者身份的签名和证书，响应端校验证书，确定是zone内请求还是zone外请求，根据名字信息设置的权限返回信息。

##### 已有Provider介绍

Simple DNS Provider：

该provider只是根据名字直接从外网DNS查询地址信息，并且将连接查询到的地址信息，获取对端身份证书，主要用于提供节点外zone信息，规则如下：

1. zone内名字直接根据名字从外网DNS查询名字的txt记录，txt信息格式为以下格式：

   ```json
   {
       "name": "service name",
       "type": "zone|node|service"
       "addr_info"： [{
       	"protocol": "string，连接协议，可取值：tcp、https、cyfs",
       	"address": "string，不同的连接协议有不同的值",
       	"port": "端口号"
   	}],
   	"api_version": "v1,服务api版本"
   	"extend": {
           "extend_key": {
    			"extend_value": "",
    		}
       },
   	"sign": "身份签名",
   	"cert_node": "证书部署的节点名字"
   }
   ```

   

2. zone外名字则根据zone名去外网DNS查询名字的txt记录，再根据zone信息中的连接地址获取该名字的地址信息。

3. 由于txt记录有长度限制，存不下身份证书，因此从dns上查询到信息之后，连接节点获取身份证书，校验信息是否可信。

Simple ETH Provider：

该provider只是根据名字直接从ETH合约中查询相关信息，规则如下：

1. 提供名字服务注册售卖合约，合约的拥有者可以更新名字相关信息，信息包括：

   

   ```json
   {
       "name": "service name",
       "type": "node|service"
       "addr_info"： [{
       	"protocol": "string，连接协议，可取值：tcp、https、cyfs",
       	"address": "string，不同的连接协议有不同的值",
       	"port": "端口号"
   	}],
   	"api_version": "v1,服务api版本"
   	"extend": {
           "extend_key": {
    			"extend_value": "",
    		}
       },
   	"sign": "身份签名",
   	"": "证书内容"
   }
   ```

2. zone内名字直接根据名字从ETH链上查询名字的地址记录，并较验信息。

3. zone外名字则根据zone名从ETH链上查询zone记录，再调用zone记录中的连接端口获取名字信息。

ETCD Provider：

1. 配置etcd运行环境，打开client-cert-auth配置

2. 配置etcd地址和校验证书，包括服务端根证书以及客户端key和证书

3. 所有名字信息都存储于etcd中，每个名字信息保存如下：

   ```json
   [{
       "name": "service name",
       "type": "zone|node|service"
       "addr_info"： [{
       	"protocol": "string，连接协议，可取值：tcp、https、cyfs",
       	"address": "string，不同的连接协议有不同的值",
       	"port": "端口号"
   	}],
   	"api_version": "v1,服务api版本",
   	"extend": {
           "extend_key": {
    			"extend_value": "",
    		}
       }
   }]
   ```

   

DNS+Decentralization Provider（暂未实现）：

该provider是在本地没有查询zone的其它节点记录时，根据zone名从外网DNS查询到Zone接入节点信息，如果本地已经有查询zone的节点信息，则直接连接相关节点获取名字信息，规则如下：

1. 为zone名配置txt记录，如example.zone，txt信息格式为：

   ```json
   {
       "name": "service name",
       "addr_info"： [{
       	"protocol": "string，连接协议，可取值：tcp、https、cyfs",
       	"address": "string，不同的连接协议有不同的值",
       	"port": "端口号"
   	}],
   	"sign": "身份签名"
   }
   ```

   

2. 此种模式下同Zone所有节点必须采用相同的通信协议。

3. 该provider提供用户设置名字信息的接口，用户根据需要可以自己设置名字信息。

4. 节点之间通过raft协议维护zone内各节点名字信息的同步。

ETH+Decentralization Provider（暂未实现）：

该provider是在本地没有查询zone的其它节点记录时，根据zone名从ETH链查询到Zone接入节点信息，如果本地已经有查询zone的节点信息，则直接连接相关节点获取名字信息，规则如下：

1. 提供名字服务注册售卖合约，合约的拥有者可以更新zone名相关信息，信息格式为：

   ```json
   {
       "name": "service name",
       "addr_info"： [{
       	"protocol": "string，连接协议，可取值：tcp、https、cyfs",
       	"address": "string，不同的连接协议有不同的值",
       	"port": "端口号"
   	}],
   	"sign": "身份签名",
   	"cert": "zone身份证书"
   }
   ```

2. 此种模式下同Zone所有节点必须采用相同的通信协议。

3. 该provider提供用户设置名字信息的接口，用户根据需要可以自己设置名字信息。

4. Zone内节点之间通过raft协议维护zone内各节点名字信息的同步。

