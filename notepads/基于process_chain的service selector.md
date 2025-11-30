# service selector
`client(runtime) -> select_node(service_name) -> access_service`

service selector是buckyos作为分布式系统的一个重要基础概念：任何分布式访问，都需要先根据目标service的selector进行处理，然后再向实际提供服务的Node发起请求。

## 核心循环
1. 用户安装服务()
2. 调度器为服务分配合适的instance
3. node_damen根据instance配置，启动service进程（镜像）
4. 启动后的进程定期汇报状态（服务租约/keep-alive)
5. 调度器根据所有信息，构建service_info（这个过程是通用的还是允许应用定制？）
6. selector（同一个应用必定使用相同类型的selector) 基于确定的service_info，根据请求选择提供服务的node

要点：
- select操作是幂等的，也就是说基于一样的请求，和一样的service_info,必然会得到一样的select结果
- 无内核重试：只要调度器没有更新service_info(故障确认)，那么select的结果不会改变。此时如果node不可用，被视作抖动而不会去进行重试应对
- 保持核心设计原则：所有人基于一致的信息进行一致操作，由一个决策者进行一次决策。
- selector看到的服务都是无状态的，有状态服务需要在得到请求后，进行二次selector
- 一个正常工作的Zone中，设计上任意两个Node之间都是可以连接成功的，却别只有速度问题

## selector的场景

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

## service的端口分配
- kernel service通常从默认端口开始选择(注意system_config的端口是固定的)
- 访问service的时候，都是基于service_info里的instance信息来访问的，因此知道其实际端口（不管是不是默认）
- zone-gateway对外提供服务时，可以配置 标准服务->端口号，比如 http:8080 ,zone-gatway知道自己暴露的服务列表（默认只有http + https)
- app根据其支持的服务，逻辑不同
    - app doc中明确说明自己在docker内使用哪个端口来实现哪些服务
    - 如果app doc实现了http服务，那么docker可以使用 -p 0:内部http 端口，随意选择，只需要正确上报就好了
    - 如果app doc实现了其它服务端口，那么docker使用 -p 服务默认端口:内部服务端口， 只有在绑定失败的时候，才会尝试从index按规则构造端口（并不断+1尝试）




