# buckyos api runtime

buckyos api runtime是使用buckyos system service的基础环境。使用基本流程如下
```
init_buckyos_api_runtime() // 收集用于login的必要信息，并创建runtime单件
runtime = get_buckyos_api_runtime()
runtime.login()? //login操作结束后会根据配置，得到后续调用系统服务的sessiont token
print("buckyos-api-runtime init success!")
```
初始化成功后，就可以通过各个service_client来访问系统的功能
```
runtime = get_buckyos_api_runtime()
sys_cfg_client = runtime.get_system_config_client()
sys_cfg_client.set("key","value")
```

需要访问buckyos的服务，都应当在进程启动的时候初始化api-runtime.

针对不同类型runtime的特性，在保持语义相同的情绪下 ，其实现在性能和兼容性之间保持平衡
- 系统内部服务之间互相访问，因为总是可以使用最新版本的buckyos-api-runtim，因此总是以最高性能模式运行
- 对应用开发者来说，即使使用老版本的api-runtime开发，也能跑在新版本的系统上，因此其实现更关注兼容性。

## 访问系统服务的方法

- 最大兼容方法：访问一个正确初始化的cyfs-gateway,然后通过其 /kapi/service_name 发送krpc请求@http，cyfs-gateway的内部会根据buckyos的系统实际情况，将请求转发到真正能处理的节点上。

- 最佳性能方法：连接system_config服务，根据当前服务的配置，决定用什么协议，向一个具体的ep(比如http://192.168.1.220:3200) 发起krpc请求。在某些配置下，这个请求会转换成对inner_service的进程调用，以提高性能。

在最大兼容方法中，cyfs- Gateway的内部实现会使用最佳性能方法来访问真正的服务。
buckyos-api-runtime还会在内部统一管理缓存（缓存策略可以配置），让某些系统调用可以安全的使用缓存，提高调用性能。

## login的具体实现

方法一，只基于zone-host进行登陆，常用于浏览器，拥有最大的兼容性

还包含一种潜在的使用：使用cyfs-gateway的dns服务，可以直接访问zone内的设备，这在某些场景下可能有用（虽然我们鼓励最大兼容模式，对系统任何资源的访问都应通过cyfs-gateway转发）。

方法二、基于zone-boot-config进行登陆，一般用于客户端软件
因为是客户软软件，所以大概率不存在127.0.0.1的cyfs-gateway. 此时可以通过zone-boot-config，以访问hostname更高性能的的连接运行在ood上的cyfs-gatway.

方法三、连接本机（127.0.0.1) 的systemconfig服务，获取boot/config后，再向verify-hub登陆,一般用于app_service
能连接本机的的cyfs-gateway,说明该进程运行在ood/server_node上。


从另一个角度，如果能通过某种方法直接访问system_config_service,并得到kv://boot/config ，那么就算是login成功了

## 为login准备必要的数据
- system_config的连接数据 (配置的越多，则初始化时所需要的时间越短)
    - zone_host_name， 通过https://zonehost/kapi/system_config 连接
    - zone_boot_confg  
    - zone_config
- 对FrameService和KernelService来说，Login时CURRENT_DEVICE_CONFIG必须已经被设置了
- 构造session-token的必要信息
    方法1：外部传入。不需要构造，只需要检验外部构造的session-token能用就好了
    方法2：有配置能用的私钥，基于私钥可以自己构造
- 对cyfs-gateway来说，需要知道当前设备DOC和PrivateKey，以支持创建rtcp tunnel. 这个依赖关系在逻辑上时可以接偶的

## 进行权限验证
大部分的系统服务，需要使用buckyos-api-runtime的enforce接口来进行标准权限认证。其通用逻辑如下
- 判断给session-token墙签名的kid是否与请求的逻辑一致
- 判断session-token是否有效：验证是否有本进程认可的公钥的签名，可信的知道请求来自有正确授权的的user_id和app_id
    一般有效的签名人是 zone-owner(kid=root), zone-ood-device(kid=), verify-hub
    当kid为none时，会访问kid=$default, 此时需要在初始化时设置，一般都是设置为verify-hub
- 通过rbac策略，判断是否能对指定资源进行操作

## 配置文件与标准目录结构

buckyos是一个典型的多进程系统，因此为了简化配置，减少配置文件不一致引起的BUG，我们鼓励使用标准的方法来完成buckyos-api-runtime的初始化
1. 在初始化的时候，由buckyos-runtime的初始化流程来管理配置的读取和设置
2. 通过buckyos-runtime，总是能得到正确的环境配置的值
3. 常规的配置优先级从高到低：
    显示指定：进程命令行
    隐式继承：读取特定的环境变量
    默认读取：读取标准环境的配置文件

标准环境有3个目录 (注意其选择是整体的，不会用优先级补足) ，Owner私钥只会存在于。Buckycli中
/BUCKYOS_ROOT/etc or /$dev_home/.buckycli or $PWD 

系统里所有的，对行为有影响的全局配置，都应通过buckyos-api-runtime来统一管理
从依赖关系上，buckyos-api-runtime所依赖的基础组件，也应在在初始化的时候由runtime统一传入配置

如未正确完成必要配置的设置，则后续调用login时会出错

### 所有的环境配置 (变量名，环境变量名，所在配置文件)

- full_app_id (owner_user_id + app_id)

- zone_did,NULL,node_identiy.json
````- NULL,BUCKY_ZONE_OWNER,node_identiy.json (did+公钥),  (move to zone_config)
- NULL,BUCKYOS_ZONE_BOOT_CONFIG,@DNS Txt Record

- CURRENT_DEVICE_CONFIG,BUCKYOS_THIS_DEVICE,node_identiy.json (移动到cyfs-gateway-lib)
- NULL,NULL,node_private_key.pem 设备私钥

- user_id,NULL,/.buckycli/user_config.json
- user_config,NULL,/.buckycli/user_config.json
- user_private_key,NULL,/.buckycli/user_private_key.pem



下面两个是login后有的
full_appid = f"{appid}-{username}"
app_token = os.getenv(f"{full_appid}_token")
- NULL,$fullid_token,NULL
- NULL,BUCKYOS_ZONE_CONFIG, 登陆成功后获得 (./did_hostname.zone.josn ) 不需要？


下面的是一些全局环境
- KNOWN_WEB3_BRIDGE_CONFIG,NULL,@machine.json （name-lib中定义且使用）
- did-doc caches （类似host文件）

## 扩展了解：系统里的几个特殊的进程

node-daemon,cyfs-gateway,system_config 是系统里比较特殊的3个进程。 系统的可用依赖这3个进程的配合，因此这3个进程无法在启动时就初始化buckyos-api-runtime,而是在一个合适的时机启用。

相对的，node-daemon,system_config的业务逻辑纯粹，后面修改的概率较低,因此可以很精细的使用buckyos-api-runtime(或不使用)，以减少在错误的状态下循环依赖的问题。

cyfs-gateaway 则要严格注意依赖关系，其本身是不依赖buckyos的基础软件（支持cyfs://), buckyos是基于cyfs-gateway本身的机制，构造了符合buckyos需求的 cyfs-gateway-service。因此cyfs-gateway本身的主体不能对buckyos产生依赖，依赖应该发生在一些具体的组件里。当这些组件加载的时候，会根据需要创建buckyos-api-runtime.

--keep tunnel 是在创建rtcp stack的时候依赖CURRENT_DEVICE

inner_service创建zone_provider的时候，依赖完整的buckyos_runtime

zone_provider还可以是tunnel selctor,用来更智能的实现selector


