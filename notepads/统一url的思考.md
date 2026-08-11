# 统一URL

为什么需要统一 URL？我们有两个需求：

AgentTool::Read
1. 我们这个 AgentTool::Read其实已经认为 URL 是一个扩展性最好的设施，可以通过指定 URL，根据一个规则得到一些确定性的、结构化的信息

另外一个就是，我们在 RBAC 做配置的时候，其实已经隐含地包含了所有类型的资源。

因为 RBAC 是面向资源的，它面对的是由本系统提供的资源，并且已经把任何资源都抽象成了一个路径。
就比如说，我们如果有一个路径是 TaskManage/TaskID。

如果 TaskID 本身也有 Event（也就是它本身会有事件），那么关于事件的权限，可以通过判断用户或特定对象对刚才讲的 TaskID 是否有权限，来进一步判断他对该 Task 的 Event 是否有权限。

从权限的视角来看，它的核心是定位到所谓的实体，也就是对象实体，以及更细节的内部路径。

它其实把路径分成了两块：
1. 只要用户对目标路径有传统的读、写、改、删这类基本逻辑（我们一般是按照 Linux 的标准逻辑）。
2. 不同的组件可以组建在一个对象上，这种能力是可以逐步、反复扩展的。

这个时候，它就可以根据这种“语义性”的权限，对应到具体资源提供者的服务上，从而对应到自己的具体RPC接口操作上去。

权限


## 几种写法的对比

http://$hostname/xxxx
obj://$did/inner_path (DID-Object协议扩展)
obj://$service_name/inner_path (需要service支持)
obj://$named_objid/inner_path 


cyfs://$objid/inner_path （优先使用，说明是zone内已经存在的object)
cyfs://$objid.$hostname/inner_path 
cyfs://$hostname/ndn/$objid/inner_path (不必要)


/local_path/
file:///local_path/ (本机文件)
file://$devicename/local_path/

/config/$inner_path 这种写法和/local_path存在潜在的冲突
kv:///$inner_path   这种写法，轻易的就扩展了schema,太不严谨
//config/$inner_path ? 
obj://config/$inner_path
等价于
obj://config.alice.bns.did/$inner_path

obj://dfs/$inner_path


## 结论

系统里所有的url都必须是合法的url,主要是下面几种

https:// 标准的http链接，注意cyfs的R-Link也可以在这里表达
file:// 标准，支持file:///local_path和 file://localhost/local_path , file://$device_id/path
cyfs:// 我们的扩展，用来获取NamedObject,URL一定指向Data
obj:// 我们的扩展，注意obj://是cyfs的超集
buckyos:// 我们的扩展，用来拉起current zone / buckyos app的特定流程


## 当用缩写路径表达时

### 
/local/path => file:///local/path 表达的是本地FS路径
//config/nodes/node1/config => obj://config/nodes/node1/config

### 我们定义的服务名

obj://config
obj://dfs
obj://taskmgr
obj://kmsg

obj://$entity_id 鼓励各种id带类型，方便路由找到，比如
obj://task_xxxx/ 系统可以识别到者是一个taskid,等价于obj://taskmgr/$taskid

> 在rbac中使用url来定义权限


