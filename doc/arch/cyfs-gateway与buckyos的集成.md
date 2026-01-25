# 新版集成问题

## NodeGateway配置
核心功能：提供本Node能访问的全部Zone内服务（运行在本Node上的服务，本Node肯定是可以访问的）
step0. 启动http stack(0.0.0.0:3180) (该成127.0.0.1会强制要求用)
step1. tcp stack收到请求,转发给node_http_server
step2. 在node_http_server中，对http请求进行处理
step3. 匹配http的host,转发请求给服务
    静态页面: 配置app_server后，直接server appid
    本机服务：直接forward 127.0.0.1:real_port
    不中转其它机器服务：直接forward: rtcp://dev_did/:3180 （单instance服务），多instance服务需要先通过 buckyos-select命令得到最终的URL
    中转其它机器服务：forward rtcp://中转节点/rtcp://dev_did/:3180 中转节点一般是SN或Zone内的Zonegatway(WLAN)


```yaml
servers:
    - id: node_gateway
      type: http
      hook_point:
        - id: main 
          prioity: 1
          blocks:
            - id: app1
              block: |
                match REQ.host "$app-host" && return "forward 127.0.0.1:$realport"
            - id: app-id2 |  
              block: |
                match REQ.host "app-host2" || pass
                buckyos-select && return "forward $ANSWER.target" 
                

stacks: 
    - id : node_gateway_tcp
      protocol: tcp
      bind: 0.0.0.0:3180
      hook_point:
        - id: main 
          prioity: 1
          blocks:
            - id : default
              block: |
                return "server node_gateway"


```

## ZoneGateway 配置
ZoneGateway一般有http stack(tcp stack)和https stack(tls stack), tls stack需要配置证书
Zone的核心配置是
step1. tls stack收到请求
step2. probe http头
step3. 根据http头，打到不同的upstream(server上)

```yaml
stacks: 
    - id : zone_gateway_http
      protocol: tcp
      bind: 0.0.0.0:80
      hook_point:
        - id: main 
          prioity: 1
          blocks:
            - id : default
              block: |
                return "server node_gateway"

stacks: 
    - id : zone_gateway_https
      protocol: tls
      bind: 0.0.0.0:443
      hook_point:
        - id: main 
          prioity: 1
          blocks:
            - id : default
              block: |
                return "server node_gateway"
```