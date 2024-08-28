# kRPC的协议设计

kPRC帮助应用开发者简单的实现一致的RPC调用。其基本思路是

## 1. 根据一个接口定义(IDL)产生必要的client-stub和server-parser代码. 使用如下

client:
```rust
rpc_client = new kPRCClient(service_url or service_name,session_token);
params = {"a":1,"b":2};
result = rpc_client.call("add",params);
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

## 3. 基于session_token的鉴权
对RPC中的session token进行验证。session_token的有效期有两种
a. 一次性有效，该session_token是和一次调用绑定的，该次调用完成后session_token失效。
b. 多次有效，通常session_token会标注一个有效期和起始的seq,验证通过后从该seq开始，直到有效期结束，都是有效的。有效期取决于服务端的配置和session_token本身携带的有效期。

session_token的验证也有两种
a. 自验证。这意味着session_token包含签名，如何能得到合适的did public key，就可以验证session_token的合法性。
b. 通过verify_hub验证，通过verify_hub来验证session_token。这类session_token通常是多次有效的。

向verify_hub申请token
verify_hub可以根据需要不断的支持新的session_token的验证方法

verify_hub也可以主动通知session_token的失效，我们鼓励通过web socket来和verify_hub保持连接实现这个功能。


## 4. session_token的安全管理
一般session_token的有效期不会超过1个周，当超过有效期后，会需要秘钥(密码)来重新获取session_token。
对重放攻击的管理：
session_token中有签发时间和有效期，因此只在这个周期内有效
同一个subject只能有一个有效的session_token，如果有新的session_token生成，旧的session_token会被废弃（自动废弃）。
通过session_token的签发时间可以用来判断谁是新的session_token
考虑到时间的误差，系统不会接受超过可信时间1小时以上的签发时间，防止发生bug
