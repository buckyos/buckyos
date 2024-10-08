# cyfs-warp

cyfs-warp 是一个基于 warp 和 rustls 的 http 服务器，用于代理和转发 http 和 https 请求。

## 使用

### 配置


## 需求整理

- 用rust实现一个http router，使用tokio，不要同时使用warp和hyper
- 支持自己定义的一个配置文件，可以把不同的url重定向到不同的upstream上，支持http和https,支持多个Host
- 一个Host如果配置了证书，那么可以同时支持http和https
- 支持websocket
- 配置文件使用toml格式
- 通过SNI来区分不同的host, 并使用不同的证书
- 通过配置，可以把一个路径配置到一个local dir上
