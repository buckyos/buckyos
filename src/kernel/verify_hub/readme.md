# verify_hub 

## 典型流程

1. node_daemon用自己的设备签名构造合适的service jwt
2. node_daemon启动app service,并将jwt传递给app service
4. [login]app service用session jwt初始化自己的krpc_client,其中krpc会与veriy_hub建立连接,并将jwt传递给verify_hub。verify_hub验证jwt，然后生成`session_token`返回给app service
5. app service用krpc_client向其它的内核服务发起请求（比如读取system_config）
6. 其它服务验证session_token，然后根据系统对app service的权限进行限制后返回结果
    session_token也是一个jwt,验证方可以根据其类型决定怎么验证。 如果是有veiry_hub签名的jwt,可以直接验证，但这种一般是用一次的
    如果是调用[verify_token]接口验证的，那么就需要与verify_hub建立连接，然后验证session_token
    对验证来说，通常账号有两个特权级别。使用session_token的是正常级别，使用signature的是高级别（类似sudo）
7. 对于可缓存的session_token，app-service应与verify_hub保持连接，随时等待verify_hub的吊销通知。


## 

## login 协议

request:
```json
{
    "method": "login",
    "params": {
        "type": "password",
        "userid": "username", 
        "appid": "appid",
        "password": "password" //password是一种加盐hash后的密码
    }
}
```

使用签名，签名的内容包括
```json
{
    "method": "login",
    "params": {
        "type": "signature",
        "iss":"did",//deviceid
        "userid": "username",
        "appid": "appid",
        "jws": "signature"
    }
}
```

使用jwt 
```json
{
    "method": "login",
    "params": {
        "type": "jwt",
        "jwt": "$jwt"
    }
}

response:

```json
{
    "result": {
        "session_token": "$session_token",
    }
}
```

## verify_token 协议

request:

```json
{
    "method": "verify_token",
    "params": {
        "session_token": "$session_token"
    }
}
```

response:

```json
{
    "result": {
        "userid": "username",
        "appid": "appid",
        "exp": 1234567890,
    }
}
```
