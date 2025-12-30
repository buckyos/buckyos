# service selector
`client(runtime) -> select_by_servcie_info(service_name) -> service_url`

service selector是buckyos作为分布式系统的一个重要基础概念：任何分布式访问，都需要先根据目标service的selector进行处理，然后再向实际提供服务的Node发起请求。

一个应用服务至少定义了一个服务
应用通过两种方法暴露其服务

### Web服务
	1. 使用短域名暴露 
	2. 使用nodeid:port暴露  
	3. 非应用服务，使用URL暴露 
	
### 其它服务
	1. 通过协议+端口暴露 
	2. 通过node:port暴露 


## 暴露服务的边界是
	1. 在特定Node上暴露
	2. 在特定Zone上暴露

Zone暴露的服务，实际上就是在ZoneGatewayNode上暴露的服务
要考虑所有的Service都能在一个Node上暴露，因此所有的Service的暴露信息是不能冲突的。调度器会保证这一点。 


## 核心循环
1. 用户安装服务(添加Spec)
2. 调度器为服务分配合适的instance (实例化)
3. node_damen根据instance配置，启动service进程（运行docker镜像）
4. 启动后的instance上报状态（服务租约/keep-alive)
5. 调度器根据服务所有的instance info，更新service info
6. selector（同一个应用必定使用相同类型的selector) 基于确定的service_info，根据请求选择提供服务的node

要点：
- select操作是幂等的，也就是说基于一样的请求，和一样的service_info,必然会得到一样的select结果
- 无内核重试：只要调度器没有更新service_info(对instance的故障进行确认)，那么select的结果不会改变。此时如果node不可用，被视作网络抖动而不会去进行应对
- cyfs-gatway总是可以以上一个状态稳定的中转流量，而不需要主动访问system-config，防止产生错误传导。
- 保持核心设计原则：所有人基于一致的信息进行一致操作，由一个决策者进行一次决策。
- selector看到的服务都是无状态的，有状态服务需要在得到请求后，进行二次selector
- 一个正常工作的Zone中，设计上任意两个Node之间都是可以连接成功的，Node的选择一般只有速度问题

### 添加Spec
通过收集UI信息后，执行下代码
```python

# hosts,path,expose_port均为UI填写，填写时会使用系统提供的函数对暴露信息进行去重
# 需要收集多少信息，取决于app-doc.servcies的定义
spec.services["www"] = {
    "expose_hosts" : {
        "filebrowser",
        "www",
    }，
    "expose_path" : "/xxxx/xxxxx/xxxx"
}

spec.services["smb"] = {
    "expose_port" : 445,
}

system_config.add_spec(spec) 
```

### 调度器基于Spec构造Instance

```python OK

expose_services = spec.services
for services in expose_services:
    # 如果返回0，就是让应用自己决定
    service.port = alloc_port(spec.app_index,service)

new_instance = {
    "expose" : sepc.expose_services
}

do_alloc_replica_instance(new_instance)
```

### 调度器根据Spec与InstanceInfo创建ServiceInfo
```rust
//用于上报给调度器的实例信息
#[derive(Serialize, Deserialize, Clone)]
pub struct ServiceInstanceReportInfo {
    pub instance_id:String,
    pub node_id:String,
    pub node_did:DID,
    pub state: ServiceInstanceState,
    //服务名->node暴露端口
    //pub service_ports:  HashMap<String,u16>, 
    pub last_update_time: u64,
    pub start_time: u64,
    pub pid: u32,
}
```

```python OK
all_instance_info = get_valid_instance_and_cacl_weight(appid)
for service in spec.services {
    service_info = {
        "selector" : {
            "type" : spec.selector_type,
            "instance" : all_instance
        },
    }
    update_service_info(get_service_id(spec.appid,service.name),service_info)
}
```

注意InstanceInfo中只有node_id, 构造service_url的逻辑依靠一个函数
```python TODO，在正式buckyos-select中实现
# 该函数会根据system_config里的zone_config,NodeInfo，构造一个确定的url
service_url = get_service_url(source_node_id,instance_info)
```

## docker 的网络服务参数确定逻辑

在启动时,node_daemon根据app的instance信息，得到host_port
docker_port根据系统服务

```python OK 
expose_services = instance.expose
for service in expose_services
    inner_port = instance.app_doc.services[service.name].port
    port_cmd += f" -p {service.port}:{inner_port}

```


## selector的类型

### 单instance
最简单的情况，只有单instance,只能选他

### 均等instance
按访问速度排序（当前runtime可以不经过测试，就得到与目标节点的测速）
在速度相同的节点中，随机选择 ，随机时会参考instance的load

### 有亲和标签
选择时，构造亲和路径，选择与亲和路径最大匹配 / 最小匹配的 Node

## selector的实现框架

- 在runtime中用rust实现，并通过命令扩展给process chain
- 通过系统升级扩展/修复 selector的逻辑


## 例子

在内核服务中访问另一个内核服务

```
fn service_fun() {
    //如果提供服务的Node是当前节点，url是127开头的
    //如果是可以直接连接的Node，且支持自加密 url是目标ip开头的 （少见）
    
    //需要RTCP : 如果是可以直接连接的Node，但需要用rtcp加密，或则不能直接连接的节点
    //使用 http://127.0.0.1:node_gateway_port/kapi/$service_name, 会在node-gateway内部执行真正的select
   
    url = runtime.select(target_service_name)
    client = new krpc_client(url) 
}


```
node-gateway的配置
```yaml

server:
    id: node-gatweay
    type: http
    process_chain: |
        call buckyos-select-service && return "forward $ANSWSER.url"

```
buckyos-select-service内部是真正的全功能实现，逻辑如下：
首先选择Node（因为幂等性，会和service_fun内选择的一致）

- 如果提供服务的Node是当前节点，http://127.0.0.1:service_port
- 如果提供服务的Node是不需要加密的可直连节点 , http://node_ip:service_port
- 如果提供的Node是需要加密的可直连节点: rtcp://node_did/:service_port
- 如果提供的Node是需要中转的节点: rtcp://中转节点/rtcp://node_did/:service_port


## 使用process chain的好处
- 允许通用pre处理，根据运维需要，针对特定节点，实现临时的selector逻辑
- 允许通过post处理，根据运维需要，临时的改变selector结果（这里可以有丰富的逻辑






