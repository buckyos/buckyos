
## 基本概念
```
resp = process_chain(req)
```
- process_chain由至少一个block组成，一个block由最少1条cmd组成
- req,resp 都由header和body构成。
- header header一定是结构化的,我们约定了一些通用的key,具体协议应在调用process chain时，构造正确的req和resp
- body根据类型，可以是结构化的或非结构化的(stream)，结构化的body可以设置kv
- 相关协议的会将结构化的req/resp转换成协议兼容的值(json/toml)。并根据resp的结果进行实际的工作（比如dispatcher要求处理链返回一个streamn url)


## process_chain的使用

下面伪代码通过一个典型的proxy实现，来介绍process_chain的关键设计
```typescript
function on_new_stream(client_stream) {
    let req = create_default_req()
    //filter阶段，还没有解析正式的req
    filter_process_chain = get_process_chain(server_id + ".filter");
    result = filter_process_chain.process(req,client_stream)
    if(result == "DROP") {
        client_stream.close()
        return;
    }

    req2 = read_http_req(client_stream)
    req = merge_req(req,req2)
    //获得和server_id一致的主处理链条
    process_chain = get_process_chain(server_id);
    //注意req在处理过程中可能被改写
    result = process_chain.process(&req)

    if(result == "DROP") {
        client_stream.close()
        return;
    }
    
    //两种有resp的情况：
    //1. process_chain本身直接处理产生了resp
    //2。process_chain计算得到了真正进行处理的upstream url
    if(result.resp) {
        return write_resp(client_stream,result.resp,req)
    } 

    if(result.upstream_url) {
        proxy_client = create_proxy_client(upstream_url)
        resp = prox_client.write_req(req)
        return write_resp(client_stream,result.resp,req) 
    }

    resp = create_inner_error_resp()
    return write_resp(client_stream,resp,req)
}

function write_resp(client_stream,resp,req) {
    post_process_chain = get_process_chain(server_id + ".post") 
    //再对resp进行一些标准处理
    result = post_process_chain.process(resp,req)
    if(result == "DROP") {
        client_stream.close()
        return;
    }
    write_http_resp(client_stream,resp)
}
```
下面是一个典型的配置（这里用XML纯粹是因为这个场景XML的表现力更好一点），功能是从host数据库中判断跳过特定的域名
```xml
<process_chain id="main_http_server.filter">
    <block type="probe">
        http-sni-probe
        match REQ_HEADER.host "*.local" && DROP
        set_lable_by_host_db REQ_HEADER.host || DROP
        have_lable REQ_HEADER.labes "direct" && DROP
    </block>

</process_chain id="main_http_server">
    <block type="process">
        match REQ_HEADER.url "appa.*/*" && EXEC usera_appa 
        match REQ_HEADER.url "appb.*/*" && EXEC usera_appb 
        match REQ_HEADER.url "appc.*/*" && EXEC usera_appc 
    </block>
</process_chain>

</process_chain id="main_http_server.post">
    <block type="rewrite">
        RESP_HEADER.cron = "*"
    </block>
</process_chain>
```
process_chain 中默认至少有一个block，block分两种：
### 命令Block （配置Block)
第一种是基于命令的，不正确的命令配置会被默认跳过，（也可配置成错误终止）
- 命令以行为单位，写起来就是标准的命令行  $cmd parm1 parm2 ，命令执行返回 成功 或 失败（有不同的错误码）
- 根据block type的不同，其可用的系统内置的cmd也不同。我们的实现会很方便我们添加cmd

- 命令执行异常会导致block（整个process_chain?)异常结束
- 在不做显示的流控制情况下，当前命令执行完成后，不管是成功还是失败，都会进入下一行。
- cmd的执行环境尽可能的接近bash(这实现起来不容易)，以降低配置人员的心智负担。基本特点如下
    - 支持环境变量
    - 可以通过 && （如果执行成功则）， || （如果支持失败则） 可以在一行里组合多个命令
    - 支持标准cli的特殊字符转义

### 脚本Block
功能强大但更难于编写，我们先不讨论。命令block的实现确定后，脚本block的实现很容易扩展。
     
## Control Flow Statements

从伪代码可以看到，native 代码主动调用了3个process_chain,我们这种直接由native code 启动的process_chain称作root process_chain. root process_chain互相之间是独立的，不存在可以从一个root process_chain跳转到另一个process_chain的过程。CONTEXT的继承是由natvie code控制的。

在root process_chain的block中，可以通过内置命令实现流控制。有限的支持流控制语句是我们的一个关键设计，在配置模式下，我们不是图灵完备的，这样不会产生死循环，并且可以对有限的流程组合进行充分的测试。
- EXEC: 从当前位置开始，用新的process chain 替换当前process chain
- GOTO: 跳转到本block的特定位置 （一个处理链可以配置最大的goto次数，跳转超过次数限制后会异常返回）
- OK: 当前process_chain成功结束
- DROP/SKIP... : process_chain 用特定错误码结束


## CMD的设计思考

- 区分对req的真实写入(rewrite) 和虚拟写入 req.header.xxx = xxx

- 管理状态
    block和process chain都可以定义只读的配置环境变量
    block和process chain都可以定义可读写的环境变量
    
- 支持网络安全（可以是iptables的替代品）

- 支持配额管理和权限管理
    对来源进行判断
    构造配额：计数并根据计数器丢弃请求

- 支持load balance    
    提取头，根据头进行处理链选择

- 支持全内核模式运行

    可以极大的提高运行性能


## 其它类似项目：
Envoy Proxy 
