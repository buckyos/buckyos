# kRPC的协议设计

kPRC帮助应用开发者简单的实现一致的RPC调用。其基本思路是

## 1. 根据一个接口定义(IDL)产生必要的client-stub和server-parser代码. 使用如下

client:
```rust
rpc_client = new kPRCClient(json_encoder);
params = {"a":1,"b":2};
rpc_client.call("add",params);
```

我们也鼓励api的提供者提供相应的定制化的client-stub,以便于更好的使用api。

```rust
my_api_client = new MyApiClient(rpc_client);
my_api_client.add(1,2);
```

server:
```rust


```

## 2. kPRC协议是简单的，即使不依赖我们的工具，任何人都可以非常简单的实现(zero-dependency)。
kPRC的核心是一个json,json里定义如下:

request:
```json
{
    "method": "add",
    "params": {
        "a": 1,
        "b": 2
    },
    "sys":[1021,"$tokenstring"]
}
```
1021是本次request的trace-id, $tokenstring是一个token,用于验证客户端的合法性。
服务器处理完成后返回如下：

response:

```json
{
    "result": 3,
    "sys":[1021]
}
```

上述协议是简单且完整的，我们不会在HTTP-Header里加入任何东西，保持协议本身的简洁和完整。

## 3. 调用链的鉴权问题(放在ACL里讨论?)