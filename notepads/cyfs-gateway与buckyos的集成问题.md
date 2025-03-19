

从分层上说，cyfs-gateway在bukcyos的分层之下,但cyfs-gateway又是我们功能的一大块，如何能进行良好的边界分离？
    - 要避免cyfs-gatway的内核组件对buckyos的依赖
    - 如有依赖，应提取到共性的lib里（通常是一个标准化的协议lib)
    - 基于cyfs-gatway实现的inner_service,可以在自己的实现里，对buckyos进行依赖（但要注意启动时序问题？）

## cyfs-gateway里可以在cyfs-dns的处理流程里，enable_zone_provider
    zone_provider的实现里，需要buckyos_api_runtime来访问system_config

## cyfs-gateway的rtcp stack初始化信息里，依赖的是基础的device did / device-document这一套
    如果cyfs-gatway不enable rtcp-stack,那么这就不是必须的
    配置文件的格式与node_daemon相同，可以直接配置使用

## cyfs-gatway是智能网关：
### buckyos 使用cyfs-gateway解决zone内 OOD/NODE之间的联通问题，这是一个刚性的需求
system-config需要依赖cyfs-gateway的机制来实现不同NAT后面的节点 组成system-config集群
    系统里的node，需要链接NAT后面的ood (目前基于SN的连接方案是zone外客户端连接NAT后的zone-gateway)
 DFS依赖cyfs-gatweay实现连接chunk server

### 通过device上的cyfs-gatway，以及device相关规则，实现智能上网行为控制
    家长控制
    智能VPN

## cyfs-gateway的ndn-router里，依赖的是ndn_manager,与Zone无关

cyfs-gateway扩展的几个层面（难度从简单到困难）
    用户不构建cyfs-gatway,使用配置文件进行扩展

    用户基于应用协议实现inner_service（provider),并构建自己的cyfs-gateway
        类似原来open-restry的路子，我们自己的服务基本都是这么在干
        扩展的机制必须是动态多态，
        parse_config(config):
            for provider_config in config.providers
                provider = create_provider_by_config(provider_config)
                process_chain.add_provider(provider)
    

    扩展cyfs-process-chain （这应该是一个内核级别的扩展？）
        扩展更多的 内置命令
    
    
        
    
    cyfs-gateway内核扩展
        支持更多的tunnel（tcp/socks/rtcp)
        支持更多的业务（应用）协议(目前是DNS/HTTP/NDN/kRPC)
            注意所有支持的应用协议都要能与process-chain机制对接
        process-chain支持新的脚本引擎，支持新的机制




每个OOD上都有cyfs-gateway
每个NODE上，都有cyfs-gateway(并保持有到zone-gatway的链接？）
每个设备上，都尽量有cyfs-gatway-socks


//Gateway需要一些必要的信息，用于正确初始化rtcp_stack
// 1. 当前设备的DeviceConfig, 私钥
// 2. 当前设备的ZoneConfig? 会用到么？
// 3. 未来cyfs-gatway是否会自主的通过system_config获取自己的配置信息？
//    cyfs-gateway要实现更新配置不重启难度比较大（或则说，这种保证在一个可扩展系统里太脆弱了）     